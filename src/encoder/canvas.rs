use image::{
    ColorType, GenericImageView, ImageBuffer, ImageResult, Pixel, PixelWithColorType, Rgb, Rgba,
};
use log::debug;
use std::io;
use std::path::{Path, PathBuf};

use crate::encoder::Encoder;
use crate::tile::Tile;
use crate::Vec2d;
use crate::ZoomError;
use std::fs::File;
use std::io::BufWriter;

type CanvasBuffer<Pix> = ImageBuffer<Pix, Vec<<Pix as Pixel>::Subpixel>>;

pub struct Canvas<Pix: Pixel = Rgba<u8>> {
    image: CanvasBuffer<Pix>,
    destination: PathBuf,
    image_writer: ImageWriter,
}

impl<Pix: Pixel> Canvas<Pix> {
    pub fn new_generic(destination: PathBuf, size: Vec2d) -> Result<Self, ZoomError> {
        Ok(Canvas {
            image: ImageBuffer::new(size.x, size.y),
            destination,
            image_writer: ImageWriter::Generic,
        })
    }

    pub fn new_jpeg(
        destination: PathBuf,
        size: Vec2d,
        quality: u8,
    ) -> Result<Canvas<Rgb<u8>>, ZoomError> {
        Ok(Canvas::<Rgb<u8>> {
            image: ImageBuffer::new(size.x, size.y),
            destination,
            image_writer: ImageWriter::Jpeg { quality },
        })
    }
}

trait FromRgba {
    fn from_rgba(rgba: Rgba<u8>) -> Self;
}

impl FromRgba for Rgba<u8> {
    fn from_rgba(rgba: Rgba<u8>) -> Self {
        rgba
    }
}

impl FromRgba for Rgb<u8> {
    fn from_rgba(rgba: Rgba<u8>) -> Self {
        rgba.to_rgb()
    }
}

impl<Pix: Pixel<Subpixel = u8> + PixelWithColorType + Send + FromRgba + 'static> Encoder
    for Canvas<Pix>
{
    fn add_tile(&mut self, tile: Tile) -> io::Result<()> {
        debug!("Copying tile data from {:?}", tile);
        let min_pos = tile.position();
        let canvas_size = self.size();
        if !min_pos.fits_inside(canvas_size) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "tile too large for image",
            ));
        }
        let max_pos = tile.bottom_right().min(canvas_size);
        let size = max_pos - min_pos;
        for y in 0..size.y {
            let canvas_y = y + min_pos.y;
            for x in 0..size.x {
                let canvas_x = x + min_pos.x;
                let p = tile.image.get_pixel(x, y);
                self.image.put_pixel(canvas_x, canvas_y, Pix::from_rgba(p));
            }
        }
        Ok(())
    }

    fn finalize(&mut self) -> io::Result<()> {
        self.image_writer
            .write(&self.image, &self.destination)
            .map_err(|e| match e {
                image::ImageError::IoError(e) => e,
                other => io::Error::new(io::ErrorKind::Other, other),
            })?;
        Ok(())
    }

    fn size(&self) -> Vec2d {
        self.image.dimensions().into()
    }
}

pub enum ImageWriter {
    Generic,
    Jpeg { quality: u8 },
}

impl ImageWriter {
    fn write<Pix: Pixel<Subpixel = u8> + PixelWithColorType>(
        &self,
        image: &CanvasBuffer<Pix>,
        destination: &Path,
    ) -> ImageResult<()> {
        match *self {
            ImageWriter::Jpeg { quality } => {
                let file = File::create(destination)?;
                let fout = &mut BufWriter::new(file);
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(fout, quality);
                encoder.encode(
                    image.as_raw(),
                    image.width(),
                    image.height(),
                    ColorType::Rgb8,
                )?;
            }
            ImageWriter::Generic => {
                image.save(destination)?;
            }
        };
        Ok(())
    }
}
