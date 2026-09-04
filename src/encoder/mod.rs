//! Output encoders: compositing tiles onto a canvas and writing image files.

use std::path::PathBuf;

use image::{DynamicImage, GenericImageView, Rgb, Rgba, SubImage};
use log::debug;

use crate::tile::{EncodedTile, Tile};
use crate::{Vec2d, ZoomError, max_size_in_rect};

pub mod canvas;
pub mod iiif_encoder;
pub mod pixel_streamer;
pub mod png_encoder;
mod retiler;
pub mod tile_buffer;
pub mod zif_tiff_encoder;

#[derive(Clone, Copy, Debug)]
pub struct SourceLevel {
    pub index: usize,
    pub size: Vec2d,
    pub scale_factor: u32,
    pub tile_size: Option<Vec2d>,
    pub has_overlapping_tiles: bool,
}

pub trait Encoder: Send + 'static {
    /// Start writing a source pyramid level.
    fn begin_level(&mut self, _level: SourceLevel) -> std::io::Result<()> {
        Ok(())
    }
    /// Add a tile to the image
    fn add_tile(&mut self, tile: Tile) -> std::io::Result<()>;
    /// Add an encoded tile to the image without decoding it.
    fn add_encoded_tile(&mut self, _tile: EncodedTile) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!(
                "{} does not support encoded tile passthrough",
                std::any::type_name::<Self>()
            ),
        ))
    }
    /// To be called when no more tile will be added
    fn finalize(&mut self) -> std::io::Result<()>;
    /// Size of the image being encoded
    fn size(&self) -> Vec2d;
}

fn encoder_for_name(
    destination: PathBuf,
    size: Vec2d,
    compression: u8,
) -> Result<Box<dyn Encoder>, ZoomError> {
    let extension = destination
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    let quality = 100u8.saturating_sub(compression);

    if extension.eq_ignore_ascii_case("png") {
        debug!("Using the streaming png encoder");
        Ok(Box::new(png_encoder::PngEncoder::new(
            destination,
            size,
            compression,
        )?))
    } else if extension.eq_ignore_ascii_case("iiif") {
        debug!("Using the iiif tiling encoder");
        Ok(Box::new(iiif_encoder::IiifEncoder::new(
            destination,
            size,
            quality,
        )?))
    } else if extension.eq_ignore_ascii_case("tiff")
        || extension.eq_ignore_ascii_case("tif")
        || extension.eq_ignore_ascii_case("zif")
    {
        debug!("Using the zif-tiff passthrough encoder");
        Ok(Box::new(zif_tiff_encoder::ZifTiffEncoder::new(
            destination,
            size,
        )?))
    } else if extension.eq_ignore_ascii_case("jpeg") || extension.eq_ignore_ascii_case("jpg") {
        debug!("Using the jpeg encoder with a quality of {quality}");
        Ok(Box::new(canvas::Canvas::<Rgb<u8>>::new_jpeg(
            destination,
            size,
            quality,
        )))
    } else {
        debug!(
            "Using the generic canvas implementation {}",
            destination.to_string_lossy()
        );
        Ok(Box::new(canvas::Canvas::<Rgba<u8>>::new_generic(
            destination,
            size,
        )))
    }
}

/// If a tile is larger than the advertised image size, then crop it to fit in the canvas
pub fn crop_tile(tile: &Tile, canvas_size: Vec2d) -> SubImage<&DynamicImage> {
    let Vec2d { x: xmax, y: ymax } = max_size_in_rect(tile.position, tile.size(), canvas_size);
    tile.image.view(0, 0, xmax, ymax)
}
