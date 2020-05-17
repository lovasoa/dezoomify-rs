use std::path::PathBuf;

use log::debug;

use crate::{Vec2d, ZoomError};
use crate::tile::Tile;

pub mod canvas;
pub mod png_encoder;
pub mod pixel_streamer;

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

/// Data structure used to store tiles until the final image size is known
pub enum TileBuffer {
    Buffer { destination: PathBuf, buffer: Vec<Tile> },
    Encoder(Box<dyn Encoder>),
}

impl TileBuffer {
    /// Create an encoder for an image of the given size at the path
    /// Errors out if the encoder cannot create files with the given extension
    /// or at the given size
    pub fn new(destination: PathBuf) -> Result<Self, ZoomError> {
        Ok(TileBuffer::Buffer {
            destination,
            buffer: vec![],
        })
    }

    pub fn set_size(&mut self, size: Vec2d) -> Result<(), ZoomError> {
        match self {
            TileBuffer::Buffer { destination, buffer } => {
                let encoder = buffer_to_encoder(destination, buffer, Some(size))?;
                std::mem::replace(self, TileBuffer::Encoder(encoder));
                Ok(())
            },
            TileBuffer::Encoder(e) => {
                assert_eq!(e.size(), size, "Unexpected size change");
                Ok(())
            },
        }
    }

    /// Add a tile to the image
    pub fn add_tile(self: &mut Self, tile: Tile) -> Result<(), ZoomError> {
        match self {
            TileBuffer::Buffer { buffer, .. } => {
                buffer.push(tile);
            },
            TileBuffer::Encoder(e) => {
                e.add_tile(tile)?;
            },
        }
        Ok(())
    }

    /// To be called when no more tile will be added
    pub fn finalize(self: &mut Self) -> Result<(), ZoomError> {
        match self {
            TileBuffer::Buffer { destination, buffer } => {
                let mut encoder = buffer_to_encoder(destination, buffer, None)?;
                encoder.finalize()?;
                std::mem::replace(self, TileBuffer::Encoder(encoder));
                Ok(())
            },
            TileBuffer::Encoder(e) => e.finalize(),
        }
    }
}

fn buffer_to_encoder(
    destination: &mut PathBuf,
    buffer: &mut Vec<Tile>,
    size: Option<Vec2d>,
) -> Result<Box<dyn Encoder>, ZoomError> {
    let size = size.unwrap_or_else(||
        buffer.iter().map(|t| t.position + t.size()).fold(
            Vec2d { x: 0, y: 0 },
            Vec2d::max,
        ));
    let mut encoder = encoder_for_name(destination.clone(), size)?;
    for tile in buffer.drain(..) {
        encoder.add_tile(tile)?;
    }
    Ok(encoder)
}