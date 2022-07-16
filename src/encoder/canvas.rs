use std::path::{PathBuf, Path};
use std::io;
use image::{GenericImage, ImageBuffer, ImageResult, PixelWithColorType, Rgba};
use log::debug;

use crate::Vec2d;
use crate::encoder::{Encoder, crop_tile};
use crate::tile::Tile;
use crate::ZoomError;
use std::io::BufWriter;
use std::fs::File;

type SubPix = u8;
type Pix = Rgba<SubPix>;
type CanvasBuffer = ImageBuffer<Pix, Vec<SubPix>>;


fn empty_buffer(size: Vec2d) -> CanvasBuffer {
    ImageBuffer::from_fn(size.x, size.y, |_, _| Pix::from([0, 0, 0, 0]))
}

pub struct Canvas {
    image: CanvasBuffer,
    destination: PathBuf,
    image_writer: ImageWriter,
}


impl Canvas {
    pub fn new(destination: PathBuf, size: Vec2d, image_writer: ImageWriter) -> Result<Self, ZoomError> {
        Ok(Canvas {
            image: empty_buffer(size),
            destination,
            image_writer,
        })
    }
}

impl Encoder for Canvas {
    fn add_tile(&mut self, tile: Tile) -> io::Result<()> {
        let sub_tile = crop_tile(&tile, self.size());
        let Vec2d { x, y } = tile.position();
        debug!("Copying tile data from {:?}", tile);
        self.image.copy_from(&*sub_tile, x, y).map_err(|_err| {
            io::Error::new(io::ErrorKind::InvalidData, "tile too large for image")
        })
    }

    fn finalize(&mut self) -> io::Result<()> {
        self.image_writer.write(&self.image, &self.destination).map_err(|e| {
            match e {
                image::ImageError::IoError(e) => e,
                other => io::Error::new(io::ErrorKind::Other, other)
            }
        })?;
        Ok(())
    }

    fn size(&self) -> Vec2d { self.image.dimensions().into() }
}

pub enum ImageWriter {
    Generic,
    Jpeg { quality: u8 },
}

impl ImageWriter {
    fn write(&self, image: &CanvasBuffer, destination: &Path) -> ImageResult<()> {
        match *self {
            ImageWriter::Jpeg { quality } => {
                let file = File::create(destination)?;
                let fout = &mut BufWriter::new(file);
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(fout, quality);
                encoder.encode(image, image.width(), image.height(), Pix::COLOR_TYPE)?;
            },
            ImageWriter::Generic => {
                image.save(destination)?;
            },
        };
        Ok(())
    }
}
