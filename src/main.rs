use custom_error::custom_error;
use image::{GenericImage, GenericImageView, ImageBuffer};
use std::str::FromStr;

use structopt::StructOpt;
use std::convert::TryFrom;
use std::convert::TryInto;
use std::fs::File;
use rayon::prelude::*;
use std::ops::Add;

mod tile_set;
mod variable;

#[derive(StructOpt, Debug)]
struct Conf {
    infile: std::path::PathBuf,
    outfile: std::path::PathBuf,
}

fn main() {
    let conf: Conf = Conf::from_args();
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
}

impl TryFrom<TileReference> for Tile {
    type Error = ZoomError;

    fn try_from(tile_reference: TileReference) -> Result<Self, Self::Error> {
        let mut buf: Vec<u8> = vec![];
        reqwest::get(&tile_reference.url)?.copy_to(&mut buf)?;
        Ok(Tile {
            image: image::load_from_memory(&buf)?,
            position: tile_reference.position,
        })
    }
}

impl FromStr for Tile {
    type Err = ZoomError;

    fn from_str(tile_str: &str) -> Result<Self, Self::Err> {
        TileReference::from_str(tile_str)?.try_into()
    }
}


fn dezoomify(conf: Conf) -> Result<(), ZoomError> {
    let file = File::open(&conf.infile)?;
    let ts: tile_set::TileSet = serde_yaml::from_reader(file)?;

    println!("Listing all tiles...");
    let tile_refs: Vec<TileReference> = ts.into_iter().collect::<Result<_, _>>()?;
    println!("Downloading tiles...");
    let tile_results: Vec<Result<Tile, _>> = tile_refs.into_par_iter()
        .map(Tile::try_from)
        .collect();

    let size = tile_results.iter().flatten()
        .map(Tile::bottom_right)
        .fold(Vec2d::default(), Vec2d::max);

    let mut canvas = Canvas::new(size);

    for tile in tile_results {
        match tile {
            Ok(tile) => {
                println!("Adding tile at x={} y={}", tile.position.x, tile.position.y);
                canvas.add_tile(&tile)?;
            }
            Err(e) => {
                eprintln!("An issue occurred with a tile: {}", e);
            }
        }
    }
    println!("Saving the image to {}...", conf.outfile.to_str().unwrap_or("(unrepresentable path)"));
    canvas.image.save(conf.outfile)?;
    Ok(())
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
    TemplateError{source: tile_set::UrlTemplateError} = "Templating error: {source}"
}