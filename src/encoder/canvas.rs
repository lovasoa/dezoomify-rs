use std::path::PathBuf;

use image::{GenericImage, GenericImageView, ImageBuffer, Pixel};
use log::debug;

use crate::{max_size_in_rect, Vec2d};
use crate::encoder::Encoder;
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
    fn add_tile(self: &mut Self, tile: Tile) -> Result<(), ZoomError> {
        let Vec2d { x: xmax, y: ymax } = max_size_in_rect(tile.position, tile.size(), self.size());
        let sub_tile = tile.image.view(0, 0, xmax, ymax);
        let Vec2d { x, y } = tile.position();
        debug!("Copying tile data from {:?}", tile);
        self.image.copy_from(&sub_tile, x, y).map_err(|_err| {
            let tile_size = tile.size();
            let size = self.size();
            ZoomError::TileCopyError {
                x,
                y,
                twidth: tile_size.x,
                theight: tile_size.y,
                width: size.x,
                height: size.y,
            }
        })
    }

    fn finalize(self: &mut Self) -> Result<(), ZoomError> {
        self.image.save(&self.destination)?;
        Ok(())
    }

    fn size(&self) -> Vec2d {
        image_size(&self.image)
    }
}

