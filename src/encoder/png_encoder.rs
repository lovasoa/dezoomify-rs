use std::fs::{File, OpenOptions};
use std::path::PathBuf;

use crate::{Vec2d, ZoomError};
use crate::tile::Tile;

use super::Encoder;
use super::pixel_streamer::PixelStreamer;

pub struct PngEncoder {
    pixel_streamer: Option<PixelStreamer<png::StreamWriter<'static, File>>>,
    size: Vec2d,
}

impl PngEncoder {
    pub fn new(destination: PathBuf, size: Vec2d) -> Result<Self, ZoomError> {
        let file = OpenOptions::new().write(true).create(true).open(destination)?;
        let mut encoder = png::Encoder::new(file, size.x, size.y);
        encoder.set_color(png::ColorType::RGB);
        encoder.set_depth(png::BitDepth::Eight);
        let writer = encoder.write_header()?.into_stream_writer();
        let pixel_streamer = Some(PixelStreamer::new(writer, size));
        Ok(PngEncoder { pixel_streamer, size })
    }
}

impl Encoder for PngEncoder {
    fn add_tile(&mut self, tile: Tile) -> Result<(), ZoomError> {
        self.pixel_streamer
            .as_mut()
            .expect("tried to add a tile in a finalized image")
            .add_tile(tile)
    }

    fn finalize(self: &mut Self) -> Result<(), ZoomError> {
        let mut pixel_streamer = self.pixel_streamer
            .take().expect("Tried to finalize an image twice");
        pixel_streamer.finalize()?;
        let writer = pixel_streamer.into_writer();
        writer.finish()?;
        Ok(())
    }

    fn size(&self) -> Vec2d {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use std::env::temp_dir;

    use image::{DynamicImage, ImageBuffer, Rgb};
    use itertools::Itertools;

    use super::*;

    #[test]
    fn test_png_create() {
        let destination = temp_dir().join("dezoomify-rs-png-test.png");
        let size = Vec2d { x: 2, y: 2 };
        let mut encoder = PngEncoder::new(destination.clone(), size).unwrap();

        encoder.add_tile(Tile {
            position: Vec2d { x: 1, y: 1 },
            image: DynamicImage::ImageRgb8(
                ImageBuffer::from_raw(1, 1, vec![1, 2, 3, ]).unwrap()
            ),
        }).unwrap();

        encoder.finalize().unwrap();
        let final_image = image::open(&destination).unwrap();
        let empty = Rgb::from([0u8, 0, 0]);
        assert_eq!(
            final_image.to_rgb().pixels().copied().collect_vec(),
            vec![empty, empty, empty, Rgb::from([1, 2, 3])]
        );
    }
}