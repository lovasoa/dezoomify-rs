use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};

use image::{GenericImage, GenericImageView, ImageBuffer};
use rayon::prelude::*;
use reqwest::{Client, header};
use serde::Deserialize;
use structopt::StructOpt;

use custom_error::custom_error;
use custom_yaml::tile_set;
use dezoomer::{Dezoomer, DezoomerError, DezoomerInput, ZoomLevels};
use dezoomer::TileReference;
use dezoomer::Vec2d;

use crate::dezoomer::ZoomLevel;

mod custom_yaml;
mod dezoomer;
mod generic;

#[derive(StructOpt, Debug)]
struct Arguments {
    input_uri: String,
    #[structopt(default_value = "dezoomified.jpg")]
    outfile: std::path::PathBuf,
    dezoomer: Option<String>
}

impl Arguments {
    fn find_dezoomer(&self) -> Result<&Dezoomer, ZoomError> {
        if let Some(name) = &self.dezoomer {
            generic::ALL_DEZOOMERS.into_iter()
                .find(|&d| d.name == name)
                .ok_or_else(|| ZoomError::NoSuchDezoomer { name: name.clone() })
        } else {
            Ok(&generic::DEZOOMER)
        }
    }
}

#[derive(Deserialize, Debug)]
struct Configuration {
    #[serde(flatten)]
    tile_set: tile_set::TileSet,
    #[serde(default = "default_headers")]
    headers: HashMap<String, String>,
}

pub fn default_headers() -> HashMap<String, String> {
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

fn fetch_uri(uri: &str, http: &Client) -> Result<Vec<u8>, ZoomError> {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        let mut contents = Vec::new();
        let mut response = http.get(uri).send()?.error_for_status()?;
        response.read_to_end(&mut contents)?;
        Ok(contents)
    } else {
        Ok(std::fs::read(uri)?)
    }
}

fn list_tiles(dezoomer: &Dezoomer, http: &Client, uri: &str)
              -> Result<ZoomLevels, ZoomError> {
    let mut i = DezoomerInput {
        uri: String::from(uri),
        contents: None,
    };
    loop {
        match dezoomer.tile_refs(&i) {
            Ok(levels) => { return Ok(levels) }
            Err(DezoomerError::NeedsData { uri }) => {
                let contents = fetch_uri(&uri, http)?;
                i.uri = uri;
                i.contents = Some(contents);
            }
            Err(e) => { return Err(e.into()) }
        }
    }
}

fn choose_level(levels: &ZoomLevels) -> Result<&ZoomLevel, ZoomError> {
    if levels.len() > 1 {
        println!("Found the following zoom levels:");
        for (i, level) in levels.iter().enumerate() {
            println!("{}. {}", i, level.name());
        }
        loop {
            print!("Which level do you want to download? ");
            let mut l = String::new();
            std::io::stdin().read_line(&mut l).expect("cannot read stdin");
            if let Ok(idx) = l.parse::<usize>() {
                if let Some(level) = levels.get(idx) {
                    return Ok(level)
                }
            }
            println!("'{}' is not a valid level number", l);
        }
    }
    levels.first().ok_or(ZoomError::NoLevels)
}

fn display_err<T, E: std::fmt::Display>(res: Result<T, E>) -> Option<T> {
    match res {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("{}", e);
            None
        }
    }
}

fn dezoomify(args: Arguments)
             -> Result<(), ZoomError> {
    let dezoomer = args.find_dezoomer()?;
    let http_client = client(default_headers())?;

    println!("Trying to locate a zoomable image...");
    let zoom_levels: Vec<ZoomLevel> = list_tiles(dezoomer, &http_client, &args.input_uri)?;
    let zoom_level = choose_level(&zoom_levels)?;

    let tile_refs: Vec<TileReference> = zoom_level.tiles().into_iter()
        .filter_map(display_err).collect();
    let total_tiles = tile_refs.len();
    let done_tiles = AtomicUsize::new(0);

    let tile_results: Vec<Option<Tile>> = tile_refs.into_par_iter()
        .map(|tile_ref| {
            let done = 1 + done_tiles.fetch_add(1, Ordering::Relaxed);
            print!("\rDownloading tiles: {}/{}", done, total_tiles);
            display_err(Tile::download(tile_ref, &http_client))
        }).collect();

    println!("\nDownloaded all tiles");

    let size = tile_results.iter().flatten()
        .map(Tile::bottom_right)
        .fold(Vec2d::default(), Vec2d::max);

    let mut canvas = Canvas::new(size);

    for tile in tile_results.iter().flatten() {
        print!("Adding tile at x={:04} y={:04}\r", tile.position.x, tile.position.y);
        canvas.add_tile(&tile)?;
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
    Dezoomer{source: DezoomerError} = "Dezoomer error: {source}",
    NoLevels = "A zoomable image was found, but it did not contain any zoom level",
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
    NoSuchDezoomer{name: String} = "No such dezoomer: {name}",
    InvalidHeaderName{source: header::InvalidHeaderName} = "Invalid header name: {source}",
    InvalidHeaderValue{source: header::InvalidHeaderValue} = "Invalid header value: {source}",
}