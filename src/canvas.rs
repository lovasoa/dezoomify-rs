use image::{GenericImage, GenericImageView, ImageBuffer, Pixel};

use crate::dezoomer::*;
use crate::Vec2d;
use crate::ZoomError;

type SubPix = u8;
type Pix = image::Rgba<SubPix>;
type CanvasBuffer = ImageBuffer<Pix, Vec<SubPix>>;

const PIXEL_SIZE: usize = std::mem::size_of::<Pix>();

const fn byte_size(area: Vec2d) -> usize {
    (area.x * area.y) as usize * PIXEL_SIZE
}

fn grow_buffer(buffer: CanvasBuffer, size: Vec2d) -> CanvasBuffer {
    let old_width = buffer.width() as usize;
    let old_height = buffer.height() as usize;
    let new_width = size.x as usize;
    assert!(new_width >= old_width);
    assert!(size.y as usize >= old_height);
    let mut raw: Vec<SubPix> = buffer.into_raw();
    raw.resize(byte_size(size), 0);
    if new_width != old_width {
        for y in (0..old_height).rev() {
            let start = y * old_width * PIXEL_SIZE;
            let end = (y + 1) * old_width * PIXEL_SIZE;
            let dest = y * new_width * PIXEL_SIZE;
            raw.copy_within(start..end, dest);
        }
    }
    ImageBuffer::from_raw(size.x, size.y, raw).unwrap()
}

fn empty_buffer(size: Vec2d) -> CanvasBuffer {
    ImageBuffer::from_fn(size.x, size.y, |_, _| Pix::from_channels(0, 0, 0, 0))
}

pub struct Canvas {
    image: CanvasBuffer,
    size: Vec2d,
    is_size_exact: bool
}

impl Canvas {
    pub fn new(size_hint: Option<Vec2d>) -> Self {
        let size = size_hint.unwrap_or(Vec2d { x: 1, y: 1 });
        let image = empty_buffer(size);
        let is_size_exact = size_hint.is_some();
        Canvas { image, size, is_size_exact }
    }

    pub fn add_tile(self: &mut Self, tile: &Tile) -> Result<(), ZoomError> {
        let new_size = tile.bottom_right().max(self.size);
        if !self.is_size_exact && new_size != self.size {
            self.size = new_size;
            let image = std::mem::replace(&mut self.image, empty_buffer(Vec2d { x: 0, y: 0 }));
            self.image = grow_buffer(image, new_size);
        }
        let Vec2d { x: xmax, y: ymax } = max_size_in_rect(tile.position, tile.size(), self.size());
        let sub_tile = tile.image.view(0, 0, xmax, ymax);

        let Vec2d { x, y } = tile.position();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grow_buffer() {
        for new_x in 1..10 {
            let mut buf = empty_buffer(Vec2d { x: 1, y: 3 });
            let p1 = Pix::from_channels(1, 2, 3, 4);
            buf.put_pixel(0, 0, p1);
            let p2 = Pix::from_channels(10, 20, 30, 40);
            buf.put_pixel(0, 1, p2);
            let resized = grow_buffer(buf, Vec2d { x: new_x, y: 3 });
            assert_eq!(&p1, resized.get_pixel(0, 0));
            assert_eq!(&p2, resized.get_pixel(0, 1));
        }
    }
}
