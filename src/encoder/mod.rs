use std::path::PathBuf;

use crate::{Vec2d, ZoomError};
use crate::tile::Tile;

pub mod canvas;

pub trait Encoder: Sized + Send + 'static {
    /// Create an encoder for an image of the given size at the path
    /// Errors out if the encoder cannot create files with the given extension
    /// or at the given size
    fn new(destination: PathBuf, size: Vec2d) -> Result<Self, ZoomError>;
    /// Add a tile to the image
    fn add_tile(self: &mut Self, tile: Tile) -> Result<(), ZoomError>;
    /// To be called when no more tile will be added
    fn finalize(self: &mut Self) -> Result<(), ZoomError>;
    /// Size of the image being encoded
    fn size(&self) -> Vec2d;
}

/// Data structure used to store tiles until the final image size is known
pub enum TileBuffer<E: Encoder> {
    Buffer { destination: PathBuf, buffer: Vec<Tile> },
    Encoder(E),
}

impl<E: Encoder> TileBuffer<E> {
    /// Create an encoder for an image of the given size at the path
    /// Errors out if the encoder cannot create files with the given extension
    /// or at the given size
    pub(crate) fn new(destination: PathBuf) -> Result<Self, ZoomError> {
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
    pub(crate) fn add_tile(self: &mut Self, tile: Tile) -> Result<(), ZoomError> {
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
                let mut encoder = buffer_to_encoder::<E>(destination, buffer, None)?;
                encoder.finalize()?;
                std::mem::replace(self, TileBuffer::Encoder(encoder));
                Ok(())
            },
            TileBuffer::Encoder(e) => e.finalize(),
        }
    }
}

fn buffer_to_encoder<E: Encoder>(
    destination: &mut PathBuf,
    buffer: &mut Vec<Tile>,
    size: Option<Vec2d>,
) -> Result<E, ZoomError> {
    let size = size.unwrap_or_else(||
        buffer.iter().map(|t| t.position + t.size()).fold(
            Vec2d { x: 0, y: 0 },
            Vec2d::max,
        ));
    let mut encoder = E::new(destination.clone(), size)?;
    for tile in buffer.drain(..) {
        encoder.add_tile(tile)?;
    }
    Ok(encoder)
}

#[cfg(test)]
mod tests {
    use image::GenericImageView;

    use super::*;

    fn my_dest() -> PathBuf { PathBuf::from("/dev/null") }

    fn my_size() -> Vec2d { Vec2d { x: 7, y: 3 } }

    fn my_tile() -> Tile {
        Tile {
            image: image::DynamicImage::new_rgb8(7 - 2, 3 - 1),
            position: Vec2d { x: 2, y: 1 },
        }
    }

    struct TestEncoder {
        destination: PathBuf,
        size: Vec2d,
        tiles: Vec<Tile>,
        finalized: bool,
    }

    impl Encoder for TestEncoder {
        fn new(destination: PathBuf, size: Vec2d) -> Result<Self, ZoomError> {
            Ok(TestEncoder {
                destination,
                size,
                tiles: vec![],
                finalized: false,
            })
        }

        fn add_tile(self: &mut Self, tile: Tile) -> Result<(), ZoomError> {
            self.tiles.push(tile);
            Ok(())
        }

        fn finalize(self: &mut Self) -> Result<(), ZoomError> {
            assert_eq!(self.finalized, false, "finalize() should not be called twice");
            self.finalized = true;
            Ok(())
        }

        fn size(&self) -> Vec2d {
            my_size()
        }
    }

    #[test]
    fn test_tilebuffer() {
        let mut b = TileBuffer::<TestEncoder>::new(my_dest()).unwrap();
        b.add_tile(my_tile()).unwrap();
        b.finalize().unwrap();
        match b {
            TileBuffer::Buffer { .. } => panic!("The tile buffer should have been finalized"),
            TileBuffer::Encoder(e) => {
                assert_eq!(e.destination, my_dest());
                assert_eq!(e.size, my_size());
                assert_eq!(e.tiles.len(), 1);
                assert_eq!(e.tiles[0].position, my_tile().position);
                assert_eq!(e.tiles[0].image.dimensions(), my_tile().image.dimensions());
                assert!(e.finalized);
            },
        }
    }
}