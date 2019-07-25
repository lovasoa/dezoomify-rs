use std::collections::HashMap;
use std::fs::File;
use std::ops::Add;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

use image::{GenericImage, GenericImageView, ImageBuffer};
use rayon::prelude::*;
use reqwest::header;
use serde::Deserialize;
use structopt::StructOpt;

use custom_error::custom_error;

mod tile_set;
mod variable;

#[derive(StructOpt, Debug)]
struct Arguments {
    infile: std::path::PathBuf,
    outfile: std::path::PathBuf,
}

#[derive(Deserialize, Debug)]
struct Configuration {
    #[serde(flatten)]
    tile_set: tile_set::TileSet,
    #[serde(default = "default_headers")]
    headers: HashMap<String, String>,
}

fn default_headers() -> HashMap<String, String> {
    serde_yaml::from_str(include_str!("default_headers.yaml")).unwrap()
}

fn main() {
    let conf: Arguments = Arguments::from_args();
    if let Err(err) = dezoomify(conf) {
        eprintln!("{}", err);
        std::process::exit(1);
    } else {
        println!("Done!");
    }
}

#[derive(Debug, PartialEq, Default, Clone, Copy)]
struct Vec2d {
    x: u32,
    y: u32,
}

impl Vec2d {
    fn max(self, other: Vec2d) -> Vec2d {
        Vec2d {
            x: self.x.max(other.x),
            y: self.y.max(other.y),
        }
    }
}

impl Add<Vec2d> for Vec2d {
    type Output = Vec2d;

    fn add(self, rhs: Vec2d) -> Self::Output {
        Vec2d { x: self.x + rhs.x, y: self.y + rhs.y }
    }
}

#[derive(Debug, PartialEq)]
pub struct TileReference {
    url: String,
    position: Vec2d,
}

impl FromStr for TileReference {
    type Err = ZoomError;

    fn from_str(tile_str: &str) -> Result<Self, Self::Err> {
        let mut parts = tile_str.split(" ");
        let make_error = || ZoomError::MalformedTileStr { tile_str: String::from(tile_str) };

        if let (Some(x), Some(y), Some(url)) = (parts.next(), parts.next(), parts.next()) {
            let x: u32 = x.parse().map_err(|_| make_error())?;
            let y: u32 = y.parse().map_err(|_| make_error())?;
            Ok(TileReference {
                url: String::from(url),
                position: Vec2d { x, y },
            })
        } else {
            Err(make_error())
        }
    }
}

fn image_size<T: GenericImageView>(image: &T) -> Vec2d {
    let (x, y) = image.dimensions();
    Vec2d { x, y }
}

struct Tile {
    image: image::DynamicImage,
    position: Vec2d,
}

impl Tile {
    fn size(&self) -> Vec2d { image_size(&self.image) }
    fn bottom_right(&self) -> Vec2d { self.size() + self.position }
    fn download(tile_reference: TileReference, client: &reqwest::Client)
                -> Result<Tile, ZoomError> {
        let mut buf: Vec<u8> = vec![];
        client.get(&tile_reference.url).send()?.copy_to(&mut buf)?;
        Ok(Tile {
            image: image::load_from_memory(&buf)?,
            position: tile_reference.position,
        })
    }
}


fn dezoomify(args: Arguments) -> Result<(), ZoomError> {
    let file = File::open(&args.infile)?;
    let Configuration {
        tile_set,
        headers
    } = serde_yaml::from_reader(file)?;

    println!("Listing all tiles...");
    let tile_refs: Vec<TileReference> = tile_set.into_iter().collect::<Result<_, _>>()?;

    let http_client = client(headers)?;

    let total_tiles = tile_refs.len();
    let done_tiles = AtomicUsize::new(0);

    let tile_results: Vec<Result<Tile, _>> = tile_refs.into_par_iter()
        .map(|tile_ref| {
            let done = 1 + done_tiles.fetch_add(1, Ordering::SeqCst);
            print!("\rDownloading tiles: {}/{}", done, total_tiles);
            Tile::download(tile_ref, &http_client)
        }).collect();

    println!("\nDownloaded all tiles");

    let size = tile_results.iter().flatten()
        .map(Tile::bottom_right)
        .fold(Vec2d::default(), Vec2d::max);

    let mut canvas = Canvas::new(size);

    for tile in tile_results {
        match tile {
            Ok(tile) => {
                print!("Adding tile at x={:04} y={:04}\r", tile.position.x, tile.position.y);
                canvas.add_tile(&tile)?;
            }
            Err(e) => {
                eprintln!("An issue occurred with a tile: {}", e);
            }
        }
    }
    println!("\nSaving the image to {}...", args.outfile.to_str().unwrap_or("(unrepresentable path)"));
    canvas.image.save(args.outfile)?;
    Ok(())
}

fn client(headers: HashMap<String, String>) -> Result<reqwest::Client, ZoomError> {
    let header_map: Result<header::HeaderMap, ZoomError> = default_headers().iter()
        .chain(headers.iter())
        .map(|(name, value)| Ok((name.parse()?, value.parse()?)))
        .collect();
    let client = reqwest::Client::builder().default_headers(header_map?).build()?;
    Ok(client)
}

struct Canvas {
    image: ImageBuffer<
        image::Rgba<u8>,
        Vec<<image::Rgba<u8> as image::Pixel>::Subpixel>
    >
}

impl Canvas {
    fn new(size: Vec2d) -> Self {
        Canvas { image: image::ImageBuffer::new(size.x, size.y) }
    }
    fn add_tile(self: &mut Self, tile: &Tile) -> Result<(), ZoomError> {
        let Vec2d { x, y } = tile.position;

        let success = self.image.copy_from(&tile.image, x, y);
        if success { Ok(()) } else {
            let Vec2d { x: twidth, y: theight } = tile.size();
            let Vec2d { x: width, y: height } = self.size();
            Err(ZoomError::TileCopyError { x, y, twidth, theight, width, height })
        }
    }
    fn size(&self) -> Vec2d { image_size(&self.image) }
}

custom_error! {
    pub ZoomError
    Networking{source: reqwest::Error} = "network error: {source}",
    Image{source: image::ImageError} = "invalid image error: {source}",
    Io{source: std::io::Error} = "Input/Output error: {source}",
    Yaml{source: serde_yaml::Error} = "Invalid YAML configuration file: {source}",
    TileCopyError{x:u32, y:u32, twidth:u32, theight:u32, width:u32, height:u32} =
                                "Unable to copy a {twidth}x{theight} tile \
                                 at position {x},{y} \
                                 on a canvas of size {width}x{height}",
    MalformedTileStr{tile_str: String} = "Malformed tile string: '{tile_str}' \
                                          expected 'x y url'",
    TemplateError{source: tile_set::UrlTemplateError} = "Templating error: {source}",
    InvalidHeaderName{source: header::InvalidHeaderName} = "Invalid header name: {source}",
    InvalidHeaderValue{source: header::InvalidHeaderValue} = "Invalid header value: {source}",
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use crate::Configuration;

    #[test]
    fn test_can_parse_example() {
        let yaml_path = format!("{}/example.yaml", env!("CARGO_MANIFEST_DIR"));
        let file = File::open(yaml_path).unwrap();
        let conf: Configuration = serde_yaml::from_reader(file).unwrap();
        assert!(conf.headers.contains_key("Referer"), "There should be a referer in the example");
    }

    #[test]
    fn test_has_default_user_agent() {
        let conf: Configuration = serde_yaml::from_str("url_template: test.com\nvariables: []").unwrap();
        assert!(conf.headers.contains_key("User-Agent"), "There should be a user agent");
    }
}