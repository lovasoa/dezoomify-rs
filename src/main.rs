use custom_error::custom_error;
use image::{GenericImage, GenericImageView, ImageBuffer};
use std::io::BufRead;
use std::str::FromStr;

use structopt::StructOpt;
use std::convert::TryFrom;
use std::convert::TryInto;

#[derive(StructOpt, Debug)]
struct Conf {
    outfile: std::path::PathBuf,
}

fn main() {
    let stdin = std::io::stdin();
    let tiles = stdin.lock()
        .lines()
        .flatten()
        .map(|s| Tile::from_str(&s))
        .flat_map(print_err);

    let conf = Conf::from_args();
    if let Err(err) = dezoomify(conf, tiles) {
        eprintln!("{}", err);
        std::process::exit(1);
    } else {
        println!("Done!");
    }
}

fn print_err<T, E: std::fmt::Display>(r: Result<T, E>) -> Result<T, E> {
    if let Err(e) = r {
        eprintln!("{}", e);
        Err(e)
    } else {
        r
    }
}

struct Vec2d {
    x: u32,
    y: u32,
}

struct TileReference {
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


struct Tile {
    image: image::DynamicImage,
    position: Vec2d,
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


fn dezoomify<T: Iterator<Item=Tile>>(conf: Conf, tiles: T) -> Result<(), ZoomError> {
    let mut canvas = Canvas::new();
    for tile in tiles { canvas.add_tile(&tile)?; }
    canvas.image.save(conf.outfile)?;
    Ok(())
}

struct Canvas {
    image: ImageBuffer<image::Rgba<u8>, Vec<<image::Rgba<u8> as image::Pixel>::Subpixel>>
}

impl Canvas {
    fn new() -> Self {
        Canvas { image: image::ImageBuffer::new(0, 0) }
    }
    fn add_tile(self: &mut Self, tile: &Tile) -> Result<(), ZoomError> {
        let x = tile.position.x;
        let y = tile.position.y;
        let twidth = tile.image.width();
        let theight = tile.image.height();
        let width = self.image.width();
        let height = self.image.height();

        let new_width = width.max(x + twidth);
        let new_height = height.max(y + theight);

        if (new_width, new_height) != (width, height) {
            self.image = image::ImageBuffer::new(new_width, new_height);
        }

        let success = self.image.copy_from(&tile.image, tile.position.x, tile.position.y);
        if success { Ok(()) } else {
            Err(ZoomError::TileCopyError { x, y, twidth, theight, width, height })
        }
    }
}

custom_error! {
    ZoomError
    Networking{source: reqwest::Error} = "network error: {source}",
    Image{source: image::ImageError} = "invalid image error: {source}",
    Io{source: std::io::Error} = "Input/Output error: {source}",
    TileCopyError{x:u32, y:u32, twidth:u32, theight:u32, width:u32, height:u32} =
                                "Unable to copy a {twidth}x{theight} tile \
                                 at position {x},{y} \
                                 on a canvas of size {width}x{height}",
    MalformedTileStr{tile_str: String} = "Malformed tile string: '{tile_str}' \
                                          expected 'x y url'",
}