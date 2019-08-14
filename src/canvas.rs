use image::{GenericImage, GenericImageView, ImageBuffer};

use crate::dezoomer::*;
use crate::Vec2d;
use crate::ZoomError;

type CanvasBuffer = ImageBuffer<image::Rgba<u8>, Vec<<image::Rgba<u8> as image::Pixel>::Subpixel>>;

pub struct Canvas {
    image: CanvasBuffer,
}

impl Canvas {
    pub fn new(size: Vec2d) -> Self {
        Canvas {
            image: image::ImageBuffer::new(size.x, size.y),
        }
    }
    pub fn add_tile(self: &mut Self, tile: &Tile) -> Result<(), ZoomError> {
        let Vec2d { x: xmax, y: ymax } = max_size_in_rect(tile.position, tile.size(), self.size());
        let sub_tile = tile.image.view(0, 0, xmax, ymax);
        let Vec2d { x, y } = tile.position;
        let success = self.image.copy_from(&sub_tile, x, y);
        if success {
            Ok(())
        } else {
            let tile_size = tile.size();
            let size = self.size();
            Err(ZoomError::TileCopyError {
                x,
                y,
                twidth: tile_size.x,
                theight: tile_size.y,
                width: size.x,
                height: size.y,
            })
        }
    }
    fn size(&self) -> Vec2d {
        image_size(&self.image)
    }
    pub fn image(&self) -> &CanvasBuffer {
        &self.image
    }
}

pub fn image_size<T: GenericImageView>(image: &T) -> Vec2d {
    let (x, y) = image.dimensions();
    Vec2d { x, y }
}

pub struct Tile {
    image: image::DynamicImage,
    position: Vec2d,
}

impl Tile {
    pub fn size(&self) -> Vec2d {
        image_size(&self.image)
    }
    pub fn bottom_right(&self) -> Vec2d {
        self.size() + self.position
    }
    pub fn download(
        zoom_level: &ZoomLevel,
        tile_reference: &TileReference,
        client: &reqwest::Client,
    ) -> Result<Tile, ZoomError> {
        let mut buf: Vec<u8> = vec![];
        let mut data = client.get(&tile_reference.url).send()?.error_for_status()?;
        data.copy_to(&mut buf)?;
        buf = zoom_level
            .post_process_tile(tile_reference, buf)
            .map_err(|source| ZoomError::PostProcessing { source })?;
        Ok(Tile {
            image: image::load_from_memory(&buf)?,
            position: tile_reference.position,
        })
    }
    pub fn position(&self) -> Vec2d {
        self.position
    }
}
