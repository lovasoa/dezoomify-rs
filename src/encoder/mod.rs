use std::path::PathBuf;

use log::debug;

use crate::{Vec2d, ZoomError};
use crate::tile::Tile;

pub mod canvas;
pub mod png_encoder;
pub mod pixel_streamer;
pub mod tile_buffer;

pub trait Encoder: Send + 'static {
    /// Add a tile to the image
    fn add_tile(self: &mut Self, tile: Tile) -> Result<(), ZoomError>;
    /// To be called when no more tile will be added
    fn finalize(self: &mut Self) -> Result<(), ZoomError>;
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