//! PNG output encoder.

use std::borrow::Cow;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;

use crate::tile::Tile;
use crate::{Vec2d, ZoomError};

use super::Encoder;
use super::pixel_streamer::PixelStreamer;

pub struct PngEncoder {
    pixel_streamer: Option<PixelStreamer<png::StreamWriter<'static, File>>>,
    file: Option<File>,
    compression: png::Compression,
    size: Vec2d,
    first_tile: bool,
}

impl PngEncoder {
    pub fn new(destination: PathBuf, size: Vec2d, compression: u8) -> Result<Self, ZoomError> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(destination)?;

        let compression_level = match compression {
            0..=19 => png::Compression::Fast,
            20..=60 => png::Compression::Balanced,
            _ => png::Compression::High,
        };

        Ok(PngEncoder {
            pixel_streamer: None,
            file: Some(file),
            compression: compression_level,
            size,
            first_tile: true,
        })
    }

    fn write_header_with_metadata(
        &mut self,
        icc_profile: Option<&Vec<u8>>,
        exif_metadata: Option<&Vec<u8>>,
    ) -> io::Result<()> {
        let file = self
            .file
            .take()
            .expect("File should be available when writing header");

        let writer = if icc_profile.is_some() || exif_metadata.is_some() {
            let mut info = png::Info::with_size(self.size.x, self.size.y);
            info.color_type = png::ColorType::Rgb;
            info.bit_depth = png::BitDepth::Eight;

            if let Some(profile) = icc_profile {
                info.icc_profile = Some(Cow::Owned(profile.clone()));
                log::debug!(
                    "Setting ICC profile in PNG header (size: {} bytes)",
                    profile.len()
                );
            }

            if let Some(exif) = exif_metadata {
                info.exif_metadata = Some(Cow::Owned(exif.clone()));
                log::debug!(
                    "Setting EXIF metadata in PNG header (size: {} bytes)",
                    exif.len()
                );
            }

            let mut encoder = png::Encoder::with_info(file, info)?;
            encoder.set_compression(self.compression);
            encoder
                .write_header()?
                .into_stream_writer_with_size(128 * 1024)?
        } else {
            let mut encoder = png::Encoder::new(file, self.size.x, self.size.y);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(self.compression);
            encoder
                .write_header()?
                .into_stream_writer_with_size(128 * 1024)?
        };

        self.pixel_streamer = Some(PixelStreamer::new(writer, self.size));
        Ok(())
    }
}

impl Encoder for PngEncoder {
    fn add_tile(&mut self, tile: Tile) -> io::Result<()> {
        if self.first_tile {
            // Write header with metadata from first tile if available
            let icc_profile = tile.icc_profile.as_ref();
            let exif_metadata = tile.exif_metadata.as_ref();

            if let Some(profile) = icc_profile {
                log::debug!(
                    "Using ICC profile from first tile (size: {} bytes)",
                    profile.len()
                );
            }

            if let Some(exif) = exif_metadata {
                log::debug!(
                    "Using EXIF metadata from first tile (size: {} bytes)",
                    exif.len()
                );
            }

            self.write_header_with_metadata(icc_profile, exif_metadata)?;
            self.first_tile = false;
        }

        self.pixel_streamer
            .as_mut()
            .expect("tried to add a tile in a finalized image")
            .add_tile(tile)
    }

    fn finalize(&mut self) -> io::Result<()> {
        // If no tiles were added, write header without metadata
        if self.first_tile {
            self.write_header_with_metadata(None, None)?;
        }

        let mut pixel_streamer = self
            .pixel_streamer
            .take()
            .expect("Tried to finalize an image twice");
        pixel_streamer.finalize()?;
        // Disabled because of https://github.com/image-rs/image-png/issues/307
        // let writer = pixel_streamer.into_writer();
        // writer.finish()?;
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
        let mut encoder = PngEncoder::new(destination.clone(), size, 1).unwrap();

        encoder
            .add_tile(
                Tile::builder()
                    .at_position(Vec2d { x: 0, y: 1 })
                    .with_image(DynamicImage::ImageRgb8(
                        ImageBuffer::from_raw(1, 1, vec![1, 2, 3]).unwrap(),
                    ))
                    .build(),
            )
            .unwrap();

        encoder.finalize().unwrap();
        let final_image = image::open(&destination).unwrap();
        let empty = Rgb::from([0u8, 0, 0]);
        assert_eq!(
            final_image.to_rgb8().pixels().copied().collect_vec(),
            vec![empty, empty, Rgb::from([1, 2, 3]), empty,]
        );
    }

    #[test]
    fn test_png_create_with_icc_profile() {
        let destination = temp_dir().join("dezoomify-rs-png-icc-test.png");
        let size = Vec2d { x: 1, y: 1 };
        let mut encoder = PngEncoder::new(destination.clone(), size, 1).unwrap();

        // Create a dummy ICC profile (simplified sRGB profile header)
        let icc_profile = vec![
            0x00, 0x00, 0x02, 0x0C, // Profile size (524 bytes)
            0x61, 0x64, 0x73, 0x70, // Signature 'adsp'
            0x00, 0x00, 0x00, 0x00, // Platform signature
            0x6D, 0x6E, 0x74, 0x72, // Device class 'mntr'
            0x52, 0x47, 0x42, 0x20, // Color space 'RGB '
        ];

        encoder
            .add_tile(
                Tile::builder()
                    .at_position(Vec2d { x: 0, y: 0 })
                    .with_image(DynamicImage::ImageRgb8(
                        ImageBuffer::from_raw(1, 1, vec![255, 0, 0]).unwrap(),
                    ))
                    .with_icc_profile(icc_profile.clone())
                    .build(),
            )
            .unwrap();

        encoder.finalize().unwrap();
        assert!(destination.exists());

        // Verify the ICC profile was actually written to the PNG
        let file = std::fs::File::open(&destination).unwrap();
        let decoder = png::Decoder::new(std::io::BufReader::new(file));
        let reader = decoder.read_info().unwrap();
        let info = reader.info();

        // Check that ICC profile exists and matches what we provided
        assert!(info.icc_profile.is_some());
        if let Some(embedded_profile) = &info.icc_profile {
            assert_eq!(embedded_profile.as_ref(), icc_profile.as_slice());
        }
    }

    #[test]
    fn test_png_create_with_exif_metadata() {
        let destination = temp_dir().join("dezoomify-rs-png-exif-test.png");
        let size = Vec2d { x: 1, y: 1 };
        let mut encoder = PngEncoder::new(destination.clone(), size, 1).unwrap();

        // Create dummy EXIF metadata (simplified EXIF header)
        let exif_metadata = vec![
            0x45, 0x78, 0x69, 0x66, // "Exif"
            0x00, 0x00, 0x4D, 0x4D, // Big-endian marker
            0x00, 0x2A, 0x00, 0x00, // TIFF header
        ];

        encoder
            .add_tile(
                Tile::builder()
                    .at_position(Vec2d { x: 0, y: 0 })
                    .with_image(DynamicImage::ImageRgb8(
                        ImageBuffer::from_raw(1, 1, vec![0, 255, 0]).unwrap(),
                    ))
                    .with_exif_metadata(exif_metadata.clone())
                    .build(),
            )
            .unwrap();

        encoder.finalize().unwrap();
        assert!(destination.exists());

        let file = std::fs::File::open(&destination).unwrap();
        let decoder = png::Decoder::new(std::io::BufReader::new(file));
        let reader = decoder.read_info().unwrap();
        let info = reader.info();
        assert_eq!(
            info.exif_metadata.as_deref(),
            Some(exif_metadata.as_slice())
        );
    }

    #[test]
    fn test_png_create_with_both_icc_and_exif() {
        let destination = temp_dir().join("dezoomify-rs-png-both-test.png");
        let size = Vec2d { x: 1, y: 1 };
        let mut encoder = PngEncoder::new(destination.clone(), size, 1).unwrap();

        let icc_profile = vec![0x61, 0x64, 0x73, 0x70]; // Simplified
        let exif_metadata = vec![0x45, 0x78, 0x69, 0x66]; // Simplified

        encoder
            .add_tile(
                Tile::builder()
                    .at_position(Vec2d { x: 0, y: 0 })
                    .with_image(DynamicImage::ImageRgb8(
                        ImageBuffer::from_raw(1, 1, vec![0, 0, 255]).unwrap(),
                    ))
                    .with_icc_profile(icc_profile.clone())
                    .with_exif_metadata(exif_metadata.clone())
                    .build(),
            )
            .unwrap();

        encoder.finalize().unwrap();
        assert!(destination.exists());

        // Verify both metadata blocks were written.
        let file = std::fs::File::open(&destination).unwrap();
        let decoder = png::Decoder::new(std::io::BufReader::new(file));
        let reader = decoder.read_info().unwrap();
        let info = reader.info();

        // ICC profile should work correctly
        assert!(info.icc_profile.is_some());
        if let Some(embedded_profile) = &info.icc_profile {
            assert_eq!(embedded_profile.as_ref(), icc_profile.as_slice());
        }

        assert_eq!(
            info.exif_metadata.as_deref(),
            Some(exif_metadata.as_slice())
        );
    }
}
