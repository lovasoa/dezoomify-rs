use std::collections::HashSet;
use std::io;
use std::path::PathBuf;

use image::{ColorType, ImageFormat, Rgba};
use log::debug;
use zif_tiff::{Codec, ColorModel};

use crate::Vec2d;
use crate::encoder::canvas::Canvas;
use crate::encoder::{Encoder, SourceLevel};
use crate::tile::{EncodedTile, Tile, load_tile_with_metadata};

pub struct ZifTiffEncoder {
    writer: zif_tiff::Writer,
    output: zif_tiff::std::FileRangeWriter,
    tile_size: Option<Vec2d>,
    codec: Option<Codec>,
    color: Option<(ColorModel, u16)>,
    occupied_tiles: HashSet<(usize, u64, u64)>,
    size: Vec2d,
    destination: PathBuf,
    declared_tile_size: Option<Vec2d>,
    fallback: Option<Canvas<Rgba<u8>>>,
    force_decoded_fallback: bool,
    current_level: usize,
}

impl ZifTiffEncoder {
    pub fn new(destination: PathBuf, size: Vec2d) -> io::Result<Self> {
        let output = zif_tiff::std::FileRangeWriter::create(&destination).map_err(to_io_error)?;
        Ok(Self {
            writer: zif_tiff::Writer::new()
                .dimensions((u64::from(size.x), u64::from(size.y)))
                // Temporary value; the first tile updates this before the writer is initialized.
                .tile_size((16, 16))
                .map_err(to_io_error)?
                .codec(Codec::Jpeg)
                .color_model(ColorModel::YCbCr)
                .channels(3)
                .map_err(to_io_error)?
                .build()
                .map_err(to_io_error)?,
            output,
            tile_size: None,
            codec: None,
            color: None,
            occupied_tiles: HashSet::new(),
            size,
            destination,
            declared_tile_size: None,
            fallback: None,
            force_decoded_fallback: false,
            current_level: 0,
        })
    }

    fn configure_from_first_tile(&mut self, tile: &EncodedTile) -> io::Result<()> {
        if self.tile_size.is_some() {
            return Ok(());
        }

        let tile_size = self.declared_tile_size.unwrap_or(tile.size);
        let codec = codec_for_format(tile.format)?;
        let (mut color_model, channels) = color_from_encoded_tile(tile)?;
        if codec == Codec::Jpeg {
            color_model = ColorModel::YCbCr;
        }
        debug!(
            "Using zif-tiff passthrough encoder: tile size {tile_size}, codec {codec:?}, color {color_model:?}/{channels} channels"
        );
        self.writer = zif_tiff::Writer::new()
            .dimensions((u64::from(self.size.x), u64::from(self.size.y)))
            .tile_size((tile_size.x, tile_size.y))
            .map_err(to_io_error)?
            .codec(codec)
            .color_model(color_model)
            .channels(channels)
            .map_err(to_io_error)?
            .pyramid()
            .build()
            .map_err(to_io_error)?;
        self.tile_size = Some(tile_size);
        self.codec = Some(codec);
        self.color = Some((color_model, channels));
        Ok(())
    }
}

impl Encoder for ZifTiffEncoder {
    fn begin_level(&mut self, level: SourceLevel) -> io::Result<()> {
        self.current_level = level.index;
        if let Some(tile_size) = level.tile_size {
            self.declared_tile_size = Some(tile_size);
        }
        self.force_decoded_fallback |= level.has_overlapping_tiles;
        Ok(())
    }

    fn add_tile(&mut self, tile: Tile) -> io::Result<()> {
        if self.tile_size.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "cannot fall back to decoded TIFF after starting zif passthrough",
            ));
        }
        if self.fallback.is_none() {
            self.fallback = Some(
                Canvas::<Rgba<u8>>::new_generic(self.destination.clone(), self.size)
                    .map_err(|err| io::Error::other(err.to_string()))?,
            );
        }
        self.fallback
            .as_mut()
            .expect("created above")
            .add_tile(tile)
    }

    fn add_encoded_tile(&mut self, tile: EncodedTile) -> io::Result<()> {
        if self.fallback.is_some() || self.force_decoded_fallback {
            let decoded = load_tile_with_metadata(tile.position, &tile.bytes)
                .map_err(|err| io::Error::other(err.to_string()))?;
            return self.add_tile(decoded);
        }
        let codec = match codec_for_format(tile.format) {
            Ok(codec) => codec,
            Err(_) => {
                let decoded = load_tile_with_metadata(tile.position, &tile.bytes)
                    .map_err(|err| io::Error::other(err.to_string()))?;
                return self.add_tile(decoded);
            }
        };
        let (mut color_model, channels) = match color_from_encoded_tile(&tile) {
            Ok(color) => color,
            Err(_) => {
                let decoded = load_tile_with_metadata(tile.position, &tile.bytes)
                    .map_err(|err| io::Error::other(err.to_string()))?;
                return self.add_tile(decoded);
            }
        };
        if codec == Codec::Jpeg {
            color_model = ColorModel::YCbCr;
        }
        let color = (color_model, channels);
        self.configure_from_first_tile(&tile)?;

        let expected_size = self.tile_size.expect("configured above");
        if Some(codec) != self.codec {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cannot mix tile codecs in one zif TIFF",
            ));
        }
        if Some(color) != self.color {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cannot mix tile color models in one zif TIFF",
            ));
        }
        if !tile.position.x.is_multiple_of(expected_size.x)
            || !tile.position.y.is_multiple_of(expected_size.y)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "tile positions must align to the zif TIFF tile grid",
            ));
        }

        let col = u64::from(tile.position.x / expected_size.x);
        let row = u64::from(tile.position.y / expected_size.y);
        if !self.occupied_tiles.insert((self.current_level, col, row)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "overlapping tiles cannot be passed through to zif TIFF",
            ));
        }

        self.output
            .apply(
                self.writer
                    .put_tile_at_level(self.current_level, (col, row), tile.bytes.as_slice())
                    .map_err(to_io_error)?,
            )
            .map_err(to_io_error)
    }

    fn finalize(&mut self) -> io::Result<()> {
        if let Some(fallback) = &mut self.fallback {
            return fallback.finalize();
        }
        if self.tile_size.is_none() {
            self.output
                .apply(self.writer.init().map_err(to_io_error)?)
                .map_err(to_io_error)?;
        }
        Ok(())
    }

    fn size(&self) -> Vec2d {
        self.size
    }
}

fn codec_for_format(format: ImageFormat) -> io::Result<Codec> {
    match format {
        ImageFormat::Jpeg => Ok(Codec::Jpeg),
        ImageFormat::Png => Ok(Codec::Png),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zif TIFF passthrough supports only JPEG and PNG input tiles",
        )),
    }
}

fn color_from_encoded_tile(tile: &EncodedTile) -> io::Result<(ColorModel, u16)> {
    match tile.color_type {
        ColorType::L8 | ColorType::L16 => Ok((ColorModel::BlackIsZero, 1)),
        ColorType::Rgb8 | ColorType::Rgb16 => Ok((ColorModel::Rgb, 3)),
        ColorType::Rgba8 | ColorType::Rgba16 | ColorType::La8 | ColorType::La16 => {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "zif TIFF passthrough does not support alpha-channel tiles",
            ))
        }
        _ if tile.color_type.has_color() => Ok((ColorModel::Rgb, 3)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zif TIFF passthrough supports grayscale or RGB-like input tiles",
        )),
    }
}

fn to_io_error(error: zif_tiff::Error) -> io::Error {
    match error {
        zif_tiff::Error::Io(e) => e,
        other => io::Error::other(other),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use image::ImageEncoder;
    use zif_tiff::std::RangeReader;

    use super::*;

    #[test]
    fn writes_input_tile_bytes_without_reencoding() {
        let mut encoded_png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut encoded_png)
            .write_image(
                &vec![255; 16 * 16 * 3],
                16,
                16,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        let bytes = Arc::new(encoded_png);
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("passthrough.tiff");
        let mut encoder = ZifTiffEncoder::new(destination.clone(), Vec2d { x: 16, y: 16 }).unwrap();

        encoder
            .add_encoded_tile(EncodedTile {
                position: Vec2d { x: 0, y: 0 },
                bytes: Arc::clone(&bytes),
                format: ImageFormat::Png,
                size: Vec2d { x: 16, y: 16 },
                color_type: ColorType::Rgb8,
            })
            .unwrap();
        encoder.finalize().unwrap();

        let mut reader = RangeReader::open(&destination).unwrap();
        let image = reader.read_zif().unwrap();
        let tile = image.level_tiles(0).unwrap().next().unwrap();
        let stored = reader.fetch(tile.range()).unwrap();
        assert_eq!(stored.bytes(), bytes.as_slice());
    }
}
