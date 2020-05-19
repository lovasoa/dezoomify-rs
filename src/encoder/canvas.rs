use std::path::PathBuf;
use std::io;
use image::{GenericImage, ImageBuffer, Pixel};
use log::debug;

use crate::Vec2d;
use crate::encoder::{Encoder, crop_tile};
use crate::tile::{image_size, Tile};
use crate::ZoomError;

type SubPix = u8;
type Pix = image::Rgba<SubPix>;
type CanvasBuffer = ImageBuffer<Pix, Vec<SubPix>>;


fn empty_buffer(size: Vec2d) -> CanvasBuffer {
    ImageBuffer::from_fn(size.x, size.y, |_, _| Pix::from_channels(0, 0, 0, 0))
}

pub struct Canvas {
    image: CanvasBuffer,
    destination: PathBuf,
}


impl Canvas {
    pub fn new(destination: PathBuf, size: Vec2d) -> Result<Self, ZoomError> {
        Ok(Canvas {
            image: empty_buffer(size),
            destination,
        })
    }
}

impl Encoder for Canvas {
    fn add_tile(self: &mut Self, tile: Tile) -> io::Result<()> {
        let sub_tile = crop_tile(&tile, self.size());
        let Vec2d { x, y } = tile.position();
        debug!("Copying tile data from {:?}", tile);
        self.image.copy_from(&sub_tile, x, y).map_err(|_err| {
            io::Error::new(io::ErrorKind::InvalidData, "tile too large for image")
        })
    }

    fn finalize(self: &mut Self) -> io::Result<()> {
        self.image.save(&self.destination).map_err(|e| {
            match e {
                image::ImageError::IoError(e) => e,
                other => io::Error::new(io::ErrorKind::Other, other)
            }
        })?;
        Ok(())
    }

    fn size(&self) -> Vec2d {
        image_size(&self.image)
    }
}

