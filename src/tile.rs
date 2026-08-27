//! Decoded tiles and the image loading helpers that produce them.

use image::{DynamicImage, GenericImageView, ImageDecoder, ImageFormat, ImageReader};
use log::{trace, warn};
use std::io::Cursor;
use std::sync::Arc;

use crate::{Vec2d, display_bytes};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedTile {
    pub position: Vec2d,
    pub bytes: Arc<Vec<u8>>,
    pub format: ImageFormat,
    pub size: Vec2d,
    pub color_type: image::ColorType,
}

#[derive(Clone)]
pub struct Tile {
    pub image: image::DynamicImage,
    pub position: Vec2d,
    pub icc_profile: Option<Vec<u8>>,
    pub exif_metadata: Option<Vec<u8>>,
}

impl Tile {
    #[must_use]
    pub fn size(&self) -> Vec2d {
        self.image.dimensions().into()
    }
    #[must_use]
    pub fn bottom_right(&self) -> Vec2d {
        self.size() + self.position
    }

    #[must_use]
    pub fn builder() -> TileBuilder {
        TileBuilder::default()
    }

    #[must_use]
    pub fn empty(position: Vec2d, size: Vec2d) -> Tile {
        Tile {
            image: DynamicImage::new_rgba8(size.x, size.y),
            position,
            icc_profile: None,
            exif_metadata: None,
        }
    }
    #[must_use]
    pub fn position(&self) -> Vec2d {
        self.position
    }
}

#[derive(Default)]
pub struct TileBuilder {
    image: Option<image::DynamicImage>,
    position: Option<Vec2d>,
    icc_profile: Option<Vec<u8>>,
    exif_metadata: Option<Vec<u8>>,
}

impl TileBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_image(mut self, image: image::DynamicImage) -> Self {
        self.image = Some(image);
        self
    }

    #[must_use]
    pub fn at_position(mut self, position: Vec2d) -> Self {
        self.position = Some(position);
        self
    }

    #[must_use]
    pub fn with_icc_profile(mut self, profile: Vec<u8>) -> Self {
        self.icc_profile = Some(profile);
        self
    }

    #[must_use]
    pub fn with_optional_icc_profile(mut self, profile: Option<Vec<u8>>) -> Self {
        self.icc_profile = profile;
        self
    }

    #[must_use]
    pub fn with_exif_metadata(mut self, metadata: Vec<u8>) -> Self {
        self.exif_metadata = Some(metadata);
        self
    }

    #[must_use]
    pub fn with_optional_exif_metadata(mut self, metadata: Option<Vec<u8>>) -> Self {
        self.exif_metadata = metadata;
        self
    }

    #[must_use]
    /// Builds the tile.
    ///
    /// # Panics
    ///
    /// Panics if no image was supplied with [`Self::with_image`].
    pub fn build(self) -> Tile {
        Tile {
            image: self.image.expect("Image is required"),
            position: self.position.unwrap_or(Vec2d { x: 0, y: 0 }),
            icc_profile: self.icc_profile,
            exif_metadata: self.exif_metadata,
        }
    }
}

/// Represents an image loaded with its associated metadata
///
/// This struct combines a decoded image with any available metadata that was
/// extracted during the loading process, such as ICC color profiles and EXIF data.
#[derive(Debug)]
pub struct ImageWithMetadata {
    pub image: DynamicImage,
    pub icc_profile: Option<Vec<u8>>,
    pub exif_metadata: Option<Vec<u8>>,
    pub format: ImageFormat,
}

type MetadataResult<T> = Result<T, image::ImageError>;

/// Reads image metadata without decoding the pixel data.
///
/// # Errors
///
/// Returns an error if the image format cannot be detected or its metadata cannot be decoded.
pub fn load_encoded_tile(position: Vec2d, bytes: Arc<Vec<u8>>) -> MetadataResult<EncodedTile> {
    let reader = ImageReader::new(Cursor::new(bytes.as_slice())).with_guessed_format()?;
    let format = reader.format().ok_or_else(unknown_format_error)?;
    let (size, color_type) = {
        let decoder = reader.into_decoder()?;
        (decoder.dimensions().into(), decoder.color_type())
    };

    Ok(EncodedTile {
        position,
        bytes,
        format,
        size,
        color_type,
    })
}

fn unknown_format_error() -> image::ImageError {
    image::ImageError::Unsupported(image::error::UnsupportedError::from_format_and_kind(
        image::error::ImageFormatHint::Unknown,
        image::error::UnsupportedErrorKind::Format(image::error::ImageFormatHint::Unknown),
    ))
}

/// Decodes a positioned tile and preserves its available metadata.
///
/// # Errors
///
/// Returns an error if the image format or pixel data cannot be decoded.
pub fn load_tile_with_metadata(position: Vec2d, bytes: &[u8]) -> MetadataResult<Tile> {
    let image_with_metadata = load_image_with_metadata(bytes)?;
    Ok(Tile::builder()
        .with_image(image_with_metadata.image)
        .at_position(position)
        .with_optional_icc_profile(image_with_metadata.icc_profile)
        .with_optional_exif_metadata(image_with_metadata.exif_metadata)
        .build())
}

/// Decodes an image and extracts its available ICC and EXIF metadata.
///
/// # Errors
///
/// Returns an error if the image format or pixel data cannot be decoded.
pub fn load_image_with_metadata(bytes: &[u8]) -> MetadataResult<ImageWithMetadata> {
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let format = reader.format().ok_or_else(unknown_format_error)?;

    // Try to get a decoder from the reader
    let mut decoder = reader.into_decoder()?;
    // Extract ICC profile first
    let icc_profile = decoder.icc_profile().unwrap_or_else(|e| {
        warn!("Failed to extract ICC profile from tile: {e}");
        None
    });

    // Extract EXIF metadata
    let exif_metadata = decoder.exif_metadata().unwrap_or_else(|e| {
        warn!("Failed to extract EXIF metadata from tile: {e}");
        None
    });

    trace!(
        "Loaded image with icc_profile {:?} and exif_metadata {:?}",
        icc_profile.as_ref().map(display_bytes),
        exif_metadata.as_ref().map(display_bytes)
    );

    // Then decode the image using the same decoder
    let image = DynamicImage::from_decoder(decoder)?;

    Ok(ImageWithMetadata {
        image,
        icc_profile,
        exif_metadata,
        format,
    })
}

impl std::fmt::Debug for Tile {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("Tile")
            .field("x", &self.position.x)
            .field("y", &self.position.y)
            .field("width", &self.image.width())
            .field("height", &self.image.height())
            .field("icc_profile", &self.icc_profile.as_ref().map(display_bytes))
            .field(
                "exif_metadata",
                &self.exif_metadata.as_ref().map(display_bytes),
            )
            .finish()
    }
}

impl PartialEq for Tile {
    fn eq(&self, other: &Self) -> bool {
        self.position == other.position
            && self.size() == other.size()
            && self.icc_profile == other.icc_profile
            && self.exif_metadata == other.exif_metadata
            && self
                .image
                .pixels()
                .all(|(x, y, pix)| other.image.get_pixel(x, y) == pix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageBuffer;

    #[test]
    fn test_load_image_with_icc_profile() {
        // Test with empty bytes (should return error)
        let empty_bytes = vec![];
        let result = load_image_with_metadata(&empty_bytes);
        assert!(result.is_err());

        // Test with invalid image data (should return error)
        let invalid_bytes = vec![0xFF, 0xD8, 0xFF, 0xE0]; // Incomplete JPEG header
        let result = load_image_with_metadata(&invalid_bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_tile_with_metadata() {
        let tile = Tile {
            image: image::DynamicImage::ImageRgb8(
                ImageBuffer::from_raw(2, 2, vec![255; 12]).unwrap(),
            ),
            position: Vec2d { x: 0, y: 0 },
            icc_profile: Some(vec![1, 2, 3, 4]), // Mock ICC profile
            exif_metadata: Some(vec![5, 6, 7, 8]), // Mock EXIF data
        };

        assert_eq!(tile.position(), Vec2d { x: 0, y: 0 });
        assert_eq!(tile.size(), Vec2d { x: 2, y: 2 });
        assert!(tile.icc_profile.is_some());
        assert!(tile.exif_metadata.is_some());
        assert_eq!(tile.icc_profile.unwrap().len(), 4);
        assert_eq!(tile.exif_metadata.unwrap().len(), 4);
    }

    #[test]
    fn test_empty_tile_has_no_metadata() {
        let tile = Tile::empty(Vec2d { x: 10, y: 10 }, Vec2d { x: 5, y: 5 });
        assert!(tile.icc_profile.is_none());
        assert!(tile.exif_metadata.is_none());
        assert_eq!(tile.position(), Vec2d { x: 10, y: 10 });
        assert_eq!(tile.size(), Vec2d { x: 5, y: 5 });
    }
}
