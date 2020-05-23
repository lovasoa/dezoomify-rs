use std::path::PathBuf;

use log::debug;

use crate::{Vec2d, ZoomError, max_size_in_rect};
use crate::tile::Tile;
use image::{SubImage, DynamicImage, GenericImageView};

pub mod canvas;
pub mod png_encoder;
pub mod pixel_streamer;
pub mod tile_buffer;

pub trait Encoder: Send + 'static {
    /// Add a tile to the image
    fn add_tile(self: &mut Self, tile: Tile) -> std::io::Result<()>;
    /// To be called when no more tile will be added
    fn finalize(self: &mut Self) -> std::io::Result<()>;
    /// Size of the image being encoded
    fn size(&self) -> Vec2d;
}

fn encoder_for_name(destination: PathBuf, size: Vec2d) -> Result<Box<dyn Encoder>, ZoomError> {
    let extension = destination.extension().unwrap_or_default();
    if extension == "png" {
        debug!("Using the streaming png encoder");
        Ok(Box::new(png_encoder::PngEncoder::new(destination, size)?))
    } else {
        debug!("Using the generic canvas implementation {}", &destination.to_string_lossy());
        Ok(Box::new(canvas::Canvas::new(destination, size)?))
    }
}

/// If a tile is larger than the advertised image size, then crop it to fit in the canvas
pub fn crop_tile(tile: &Tile, canvas_size: Vec2d) -> SubImage<&DynamicImage> {
    let Vec2d { x: xmax, y: ymax } = max_size_in_rect(tile.position, tile.size(), canvas_size);
    tile.image.view(0, 0, xmax, ymax)
}