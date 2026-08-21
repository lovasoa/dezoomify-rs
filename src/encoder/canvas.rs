//! In-memory RGBA canvas that composited tiles are read from when encoding.

use image::{
    ExtendedColorType, GenericImageView, ImageBuffer, ImageEncoder, ImageResult, Pixel,
    PixelWithColorType, Rgb, Rgba,
};
use log::debug;
use std::io;
use std::path::{Path, PathBuf};

use crate::Vec2d;
use crate::encoder::Encoder;
use crate::tile::Tile;
use std::fs::File;
use std::io::BufWriter;

type CanvasBuffer<Pix> = ImageBuffer<Pix, Vec<<Pix as Pixel>::Subpixel>>;

pub struct Canvas<Pix: Pixel = Rgba<u8>> {
    image: CanvasBuffer<Pix>,
    destination: PathBuf,
    image_writer: ImageWriter,
    icc_profile: Option<Vec<u8>>,
}

impl<Pix: Pixel> Canvas<Pix> {
    pub fn new_generic(destination: PathBuf, size: Vec2d) -> Self {
        Canvas {
            image: ImageBuffer::new(size.x, size.y),
            destination,
            image_writer: ImageWriter::Generic,
            icc_profile: None,
        }
    }

    pub fn new_jpeg(destination: PathBuf, size: Vec2d, quality: u8) -> Canvas<Rgb<u8>> {
        Canvas::<Rgb<u8>> {
            image: ImageBuffer::new(size.x, size.y),
            destination,
            image_writer: ImageWriter::Jpeg { quality },
            icc_profile: None,
        }
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

#[allow(clippy::panicking_unwrap)] // https://github.com/rust-lang/rust-clippy/issues/16188
impl<Pix: Pixel<Subpixel = u8> + PixelWithColorType + Send + FromRgba + 'static> Encoder
    for Canvas<Pix>
{
    fn add_tile(&mut self, tile: Tile) -> io::Result<()> {
        debug!("Copying tile data from {tile:?}");

        // Capture ICC profile from the first tile that has one
        if self.icc_profile.is_none() && tile.icc_profile.is_some() {
            self.icc_profile.clone_from(&tile.icc_profile);
            debug!(
                "Captured ICC profile from tile (size: {} bytes)",
                self.icc_profile.as_ref().unwrap().len()
            );
        }

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
            .write(&self.image, &self.destination, self.icc_profile.as_deref())
            .map_err(|e| match e {
                image::ImageError::IoError(e) => e,
                other => io::Error::other(other),
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
        icc_profile: Option<&[u8]>,
    ) -> ImageResult<()> {
        match *self {
            ImageWriter::Jpeg { quality } => {
                let file = File::create(destination)?;
                let fout = &mut BufWriter::new(file);
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(fout, quality);

                // Set ICC profile if available
                if let Some(profile) = icc_profile {
                    if let Err(e) = encoder.set_icc_profile(profile.to_vec()) {
                        debug!("Failed to set ICC profile for JPEG: {e}");
                    } else {
                        debug!("Applied ICC profile to JPEG output");
                    }
                }

                encoder.encode(
                    image.as_raw(),
                    image.width(),
                    image.height(),
                    ExtendedColorType::Rgb8,
                )?;
            }
            ImageWriter::Generic => {
                // For generic format, we need to handle ICC profiles based on the file extension
                if let Some(profile) = icc_profile {
                    Self::write_with_icc_profile(image, destination, profile)?;
                } else {
                    image.save(destination)?;
                }
            }
        }
        Ok(())
    }

    fn write_with_icc_profile<Pix: Pixel<Subpixel = u8> + PixelWithColorType>(
        image: &CanvasBuffer<Pix>,
        destination: &Path,
        icc_profile: &[u8],
    ) -> ImageResult<()> {
        let extension = destination
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        match extension.to_lowercase().as_str() {
            "png" => {
                Self::encode_with_icc_profile::<Pix, image::codecs::png::PngEncoder<BufWriter<File>>>(
                    image,
                    destination,
                    icc_profile,
                    image::codecs::png::PngEncoder::new,
                    "PNG",
                )
            }
            "tiff" | "tif" => Self::encode_with_icc_profile::<
                Pix,
                image::codecs::tiff::TiffEncoder<BufWriter<File>>,
            >(
                image,
                destination,
                icc_profile,
                image::codecs::tiff::TiffEncoder::new,
                "TIFF",
            ),
            "webp" => Self::encode_with_icc_profile::<
                Pix,
                image::codecs::webp::WebPEncoder<BufWriter<File>>,
            >(
                image,
                destination,
                icc_profile,
                image::codecs::webp::WebPEncoder::new_lossless,
                "WebP",
            ),
            _ => {
                // For other formats, fall back to the standard save method
                debug!("ICC profile not supported for format: {extension}");
                image.save(destination)
            }
        }
    }

    fn encode_with_icc_profile<Pix, E>(
        image: &CanvasBuffer<Pix>,
        destination: &Path,
        icc_profile: &[u8],
        encoder_factory: fn(BufWriter<File>) -> E,
        format_name: &str,
    ) -> ImageResult<()>
    where
        Pix: Pixel<Subpixel = u8> + PixelWithColorType,
        E: ImageEncoder,
    {
        let file = File::create(destination)?;
        let fout = BufWriter::new(file);
        let mut encoder = encoder_factory(fout);

        if let Err(e) = encoder.set_icc_profile(icc_profile.to_owned()) {
            debug!("Failed to set ICC profile for {format_name}: {e}");
        } else {
            debug!("Applied ICC profile to {format_name} output");
        }

        encoder.write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            Pix::COLOR_TYPE,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};
    use std::env::temp_dir;

    #[test]
    fn test_canvas_captures_icc_profile() {
        let destination = temp_dir().join("test_icc_canvas.png");
        let size = Vec2d { x: 2, y: 2 };
        let mut canvas = Canvas::<Rgba<u8>>::new_generic(destination, size);

        // Verify canvas starts with no ICC profile
        assert!(canvas.icc_profile.is_none());

        // Add a tile with an ICC profile
        let tile_with_profile = Tile::builder()
            .with_image(image::DynamicImage::ImageRgba8(
                ImageBuffer::from_raw(1, 1, vec![255, 0, 0, 255]).unwrap(),
            ))
            .at_position(Vec2d { x: 0, y: 0 })
            .with_icc_profile(vec![1, 2, 3, 4, 5])
            .build();

        canvas.add_tile(tile_with_profile).unwrap();

        // Verify canvas now has the ICC profile
        assert!(canvas.icc_profile.is_some());
        assert_eq!(canvas.icc_profile.unwrap().len(), 5);
    }

    #[test]
    fn test_canvas_ignores_later_icc_profiles() {
        let destination = temp_dir().join("test_icc_priority.png");
        let size = Vec2d { x: 2, y: 2 };
        let mut canvas = Canvas::<Rgba<u8>>::new_generic(destination, size);

        // Add first tile with ICC profile
        let first_tile = Tile::builder()
            .with_image(image::DynamicImage::ImageRgba8(
                ImageBuffer::from_raw(1, 1, vec![255, 0, 0, 255]).unwrap(),
            ))
            .at_position(Vec2d { x: 0, y: 0 })
            .with_icc_profile(vec![1, 2, 3])
            .build();

        canvas.add_tile(first_tile).unwrap();
        let first_profile = canvas.icc_profile.clone();

        // Add second tile with different ICC profile
        let second_tile = Tile::builder()
            .with_image(image::DynamicImage::ImageRgba8(
                ImageBuffer::from_raw(1, 1, vec![0, 255, 0, 255]).unwrap(),
            ))
            .at_position(Vec2d { x: 1, y: 0 })
            .with_icc_profile(vec![4, 5, 6, 7])
            .build();

        canvas.add_tile(second_tile).unwrap();

        // Verify canvas still has the first ICC profile
        assert_eq!(canvas.icc_profile, first_profile);
        assert_eq!(canvas.icc_profile.unwrap().len(), 3);
    }
}
