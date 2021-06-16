use std::path::PathBuf;

use image::{DynamicImage, GenericImageView, SubImage};
use log::debug;

use crate::{max_size_in_rect, Vec2d, ZoomError};
use crate::encoder::canvas::ImageWriter;
use crate::tile::Tile;

pub mod canvas;
pub mod png_encoder;
pub mod pixel_streamer;
pub mod tile_buffer;
pub mod iiif_encoder;
mod retiler;

pub trait Encoder: Send + 'static {
    /// Add a tile to the image
    fn add_tile(&mut self, tile: Tile) -> std::io::Result<()>;
    /// To be called when no more tile will be added
    fn finalize(&mut self) -> std::io::Result<()>;
    /// Size of the image being encoded
    fn size(&self) -> Vec2d;
}

fn encoder_for_name(destination: PathBuf, size: Vec2d, compression: u8) -> Result<Box<dyn Encoder>, ZoomError> {
    let extension = destination.extension().unwrap_or_default();
    let quality = 100u8.saturating_sub(compression);

    if extension == "png" {
        debug!("Using the streaming png encoder");
        Ok(Box::new(png_encoder::PngEncoder::new(destination, size, compression)?))
    } else if extension == "iiif" {
        debug!("Using the iiif tiling encoder");
        Ok(Box::new(iiif_encoder::IiifEncoder::new(destination, size, quality)?))
    } else if extension == "jpeg" || extension == "jpg" {
        debug!("Using the jpeg encoder with a quality of {}", quality);
        let image_writer = ImageWriter::Jpeg { quality };
        Ok(Box::new(canvas::Canvas::new(destination, size, image_writer)?))
    } else {
        debug!("Using the generic canvas implementation {}", &destination.to_string_lossy());
        Ok(Box::new(canvas::Canvas::new(destination, size, ImageWriter::Generic)?))
    }
}

/// If a tile is larger than the advertised image size, then crop it to fit in the canvas
pub fn crop_tile(tile: &Tile, canvas_size: Vec2d) -> SubImage<&DynamicImage> {
    let Vec2d { x: xmax, y: ymax } = max_size_in_rect(tile.position, tile.size(), canvas_size);
    tile.image.view(0, 0, xmax, ymax)
}
