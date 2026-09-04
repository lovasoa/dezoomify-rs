//! ZIF (zoomable image format) TIFF output encoder.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use image::{ColorType, ImageFormat, Rgba};
use log::debug;
use zif_tiff::{Codec, ColorModel, LevelConfig};

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
    jpeg: Option<JpegTileInfo>,
    occupied_tiles: HashSet<(usize, u64, u64)>,
    size: Vec2d,
    destination: PathBuf,
    declared_tile_size: Option<Vec2d>,
    fallback: Option<Canvas<Rgba<u8>>>,
    fallback_destination: Option<PathBuf>,
    force_decoded_fallback: bool,
    source_pyramid: bool,
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
            jpeg: None,
            occupied_tiles: HashSet::new(),
            size,
            destination,
            declared_tile_size: None,
            fallback: None,
            fallback_destination: None,
            force_decoded_fallback: false,
            source_pyramid: false,
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
        let jpeg = if codec == Codec::Jpeg {
            Some(parse_jpeg_tile_info(&tile.bytes)?)
        } else {
            None
        };
        if codec == Codec::Jpeg && channels == 3 {
            color_model = ColorModel::YCbCr;
        }
        let ycbcr_subsampling = jpeg.map(|info| info.subsampling);
        debug!(
            "Using zif-tiff passthrough encoder: tile size {tile_size}, codec {codec:?}, color {color_model:?}/{channels} channels, JPEG subsampling {ycbcr_subsampling:?}"
        );
        let mut builder = zif_tiff::Writer::new()
            .dimensions((u64::from(self.size.x), u64::from(self.size.y)))
            .tile_size((tile_size.x, tile_size.y))
            .map_err(to_io_error)?
            .codec(codec)
            .color_model(color_model)
            .channels(channels)
            .map_err(to_io_error)?;
        if let Some(subsampling) = ycbcr_subsampling {
            builder = if subsampling == (1, 1) || subsampling == (2, 2) {
                builder
                    .ycbcr_subsampling(subsampling)
                    .map_err(to_io_error)?
            } else {
                builder
                    .preserve_nonstandard_ycbcr_subsampling(subsampling)
                    .map_err(to_io_error)?
            };
        }
        self.writer = builder.build().map_err(to_io_error)?;
        self.tile_size = Some(tile_size);
        self.codec = Some(codec);
        self.color = Some((color_model, channels));
        self.jpeg = jpeg;
        Ok(())
    }

    fn add_sub_level(&mut self, index: usize, level: &SourceLevel) -> io::Result<()> {
        let sf = u64::from(level.scale_factor);
        let dims = (
            u64::from(self.size.x).div_ceil(sf),
            u64::from(self.size.y).div_ceil(sf),
        );
        let ts = self
            .declared_tile_size
            .or(self.tile_size)
            .expect("tile size is set before sub-levels arrive");
        debug!("Adding level {index} with dimensions {dims:?}, tile size {ts:?}");
        let batch = self
            .writer
            .add_level(
                index,
                LevelConfig::new(dims, (ts.x, ts.y)).map_err(to_io_error)?,
            )
            .map_err(to_io_error)?;
        self.output.apply(batch).map_err(to_io_error)
    }

    fn decode_or_fall_back(&mut self, tile: &EncodedTile) -> io::Result<()> {
        if is_zif_destination(&self.destination) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "zif output requires passthrough-compatible JPEG tiles; use a .tiff extension for decoded fallback output",
            ));
        }
        if self.source_pyramid && self.tile_size.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cannot switch from zif passthrough to decoded TIFF after writing tiles",
            ));
        }
        let decoded = load_tile_with_metadata(tile.position, &tile.bytes)
            .map_err(|err| io::Error::other(err.to_string()))?;
        self.add_tile(decoded)
    }
}

impl Encoder for ZifTiffEncoder {
    fn begin_level(&mut self, level: SourceLevel) -> io::Result<()> {
        self.source_pyramid = true;
        self.current_level = level.index;
        if self.fallback.is_some() {
            return Ok(());
        }
        if let Some(tile_size) = level.tile_size {
            self.declared_tile_size = Some(tile_size);
        }
        self.force_decoded_fallback |= level.has_overlapping_tiles;
        if self.tile_size.is_some() {
            self.add_sub_level(level.index, &level)?;
        }
        Ok(())
    }

    fn add_tile(&mut self, tile: Tile) -> io::Result<()> {
        if self.source_pyramid && self.fallback.is_some() && self.current_level != 0 {
            return Ok(());
        }
        if let Some(fallback) = &mut self.fallback {
            return fallback.add_tile(tile);
        }
        if self.tile_size.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "cannot fall back to decoded TIFF after starting zif passthrough",
            ));
        }
        if self.fallback.is_none() {
            if is_zif_destination(&self.destination) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "zif output requires passthrough-compatible JPEG tiles; use a .tiff extension for decoded fallback output",
                ));
            }
            let fallback_destination = decoded_fallback_destination(&self.destination);
            self.fallback = Some(Canvas::<Rgba<u8>>::new_generic(
                fallback_destination.clone(),
                self.size,
            ));
            self.fallback_destination = Some(fallback_destination);
        }
        self.fallback
            .as_mut()
            .expect("created above")
            .add_tile(tile)
    }

    fn add_encoded_tile(&mut self, tile: EncodedTile) -> io::Result<()> {
        if self.source_pyramid && self.fallback.is_some() && self.current_level != 0 {
            return Ok(());
        }
        if self.fallback.is_some() || self.force_decoded_fallback {
            return self.decode_or_fall_back(&tile);
        }
        let Ok(codec) = codec_for_format(tile.format) else {
            return self.decode_or_fall_back(&tile);
        };
        let Ok((mut color_model, channels)) = color_from_encoded_tile(&tile) else {
            return self.decode_or_fall_back(&tile);
        };
        if codec == Codec::Jpeg && channels == 3 {
            color_model = ColorModel::YCbCr;
        }
        let jpeg = if codec == Codec::Jpeg {
            match parse_jpeg_tile_info(&tile.bytes) {
                Ok(jpeg) => Some(jpeg),
                Err(err) if self.source_pyramid && self.tile_size.is_some() => return Err(err),
                Err(_) => return self.decode_or_fall_back(&tile),
            }
        } else {
            None
        };
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
        if jpeg != self.jpeg {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cannot mix JPEG sampling layouts in one zif TIFF",
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
            fallback.finalize()?;
            if let Some(fallback_destination) = &self.fallback_destination
                && fallback_destination != &self.destination
            {
                std::fs::rename(fallback_destination, &self.destination)?;
            }
            return Ok(());
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct JpegTileInfo {
    subsampling: (u16, u16),
}

fn parse_jpeg_tile_info(bytes: &[u8]) -> io::Result<JpegTileInfo> {
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid JPEG tile",
        ));
    }

    let mut pos = 2;
    while pos + 4 <= bytes.len() {
        while pos < bytes.len() && bytes[pos] == 0xff {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let marker = bytes[pos];
        pos += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        if pos + 2 > bytes.len() {
            break;
        }
        let len = usize::from(u16::from_be_bytes([bytes[pos], bytes[pos + 1]]));
        if len < 2 || pos + len > bytes.len() {
            break;
        }
        let segment = &bytes[pos + 2..pos + len];
        if let 0xc0..=0xc2 = marker {
            return parse_jpeg_start_of_frame(segment);
        }
        pos += len;
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "JPEG tile has no supported start-of-frame marker",
    ))
}

fn parse_jpeg_start_of_frame(segment: &[u8]) -> io::Result<JpegTileInfo> {
    if segment.len() < 6 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "truncated JPEG start-of-frame segment",
        ));
    }
    let components = usize::from(segment[5]);
    if components == 0 || segment.len() < 6 + components * 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid JPEG component table",
        ));
    }

    let mut max_h = 1u16;
    let mut max_v = 1u16;
    for component in segment[6..6 + components * 3].as_chunks::<3>().0 {
        let sampling = component[1];
        max_h = max_h.max(u16::from(sampling >> 4));
        max_v = max_v.max(u16::from(sampling & 0x0f));
    }
    if max_h == 0 || max_v == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid JPEG sampling factors",
        ));
    }

    Ok(JpegTileInfo {
        subsampling: (max_h, max_v),
    })
}

fn decoded_fallback_destination(destination: &Path) -> PathBuf {
    if is_zif_destination(destination) {
        destination.with_extension("tiff")
    } else {
        destination.to_owned()
    }
}

fn is_zif_destination(destination: &Path) -> bool {
    destination
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zif"))
}

fn codec_for_format(format: ImageFormat) -> io::Result<Codec> {
    match format {
        ImageFormat::Jpeg => Ok(Codec::Jpeg),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zif TIFF passthrough supports only JPEG input tiles",
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
        let mut encoded = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 95)
            .write_image(
                &vec![128; 16 * 16 * 3],
                16,
                16,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        let bytes = Arc::new(encoded);
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("passthrough.tiff");
        let mut output_encoder =
            ZifTiffEncoder::new(destination.clone(), Vec2d { x: 16, y: 16 }).unwrap();

        output_encoder
            .add_encoded_tile(EncodedTile {
                position: Vec2d { x: 0, y: 0 },
                bytes: Arc::clone(&bytes),
                format: ImageFormat::Jpeg,
                size: Vec2d { x: 16, y: 16 },
                color_type: ColorType::Rgb8,
            })
            .unwrap();
        output_encoder.finalize().unwrap();

        let mut reader = RangeReader::open(&destination).unwrap();
        let image = reader.read_zif().unwrap();
        let tile = image.level_tiles(0).unwrap().next().unwrap();
        let stored = reader.fetch(tile.range()).unwrap();
        assert_eq!(stored.bytes(), bytes.as_slice());
    }

    #[test]
    fn writes_grayscale_jpeg_tiles_without_changing_their_channel_count() {
        let mut encoded = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 95)
            .write_image(&vec![128; 16 * 16], 16, 16, image::ExtendedColorType::L8)
            .unwrap();
        let bytes = Arc::new(encoded);
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("grayscale.tiff");
        let mut output_encoder =
            ZifTiffEncoder::new(destination.clone(), Vec2d { x: 16, y: 16 }).unwrap();

        output_encoder
            .add_encoded_tile(EncodedTile {
                position: Vec2d::default(),
                bytes: Arc::clone(&bytes),
                format: ImageFormat::Jpeg,
                size: Vec2d { x: 16, y: 16 },
                color_type: ColorType::L8,
            })
            .unwrap();
        output_encoder.finalize().unwrap();

        let mut reader = RangeReader::open(&destination).unwrap();
        let image = reader.read_zif().unwrap();
        assert_eq!(image.channels(), 1);
        let tile = image.level_tiles(0).unwrap().next().unwrap();
        assert_eq!(
            reader.fetch(tile.range()).unwrap().bytes(),
            bytes.as_slice()
        );
    }

    #[test]
    fn writes_multiple_source_levels() {
        let mut encoded = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 95)
            .write_image(
                &vec![128; 16 * 16 * 3],
                16,
                16,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        let bytes = Arc::new(encoded);
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("pyramid.tiff");
        let size = Vec2d { x: 32, y: 32 };
        let tile_size = Vec2d { x: 16, y: 16 };
        let mut output_encoder = ZifTiffEncoder::new(destination.clone(), size).unwrap();

        for (index, scale_factor) in [1, 2].into_iter().enumerate() {
            output_encoder
                .begin_level(SourceLevel {
                    index,
                    size,
                    scale_factor,
                    tile_size: Some(tile_size),
                    has_overlapping_tiles: false,
                })
                .unwrap();
            output_encoder
                .add_encoded_tile(EncodedTile {
                    position: Vec2d::default(),
                    bytes: Arc::clone(&bytes),
                    format: ImageFormat::Jpeg,
                    size: tile_size,
                    color_type: ColorType::Rgb8,
                })
                .unwrap();
        }
        output_encoder.finalize().unwrap();

        let mut reader = RangeReader::open(&destination).unwrap();
        let image = reader.read_zif().unwrap();
        for level in 0..2 {
            let tile = image.level_tiles(level).unwrap().next().unwrap();
            let stored = reader.fetch(tile.range()).unwrap();
            assert_eq!(stored.bytes(), bytes.as_slice());
        }
    }

    #[test]
    fn parses_jpeg_subsampling_from_baseline_frame() {
        let jpeg = minimal_jpeg_with_sof(0xc0, 0x11);
        assert_eq!(
            parse_jpeg_tile_info(&jpeg).unwrap(),
            JpegTileInfo {
                subsampling: (1, 1)
            }
        );

        let jpeg = minimal_jpeg_with_sof(0xc0, 0x22);
        assert_eq!(
            parse_jpeg_tile_info(&jpeg).unwrap(),
            JpegTileInfo {
                subsampling: (2, 2)
            }
        );
    }

    #[test]
    fn parses_jpeg_subsampling_from_progressive_frame() {
        let jpeg = minimal_jpeg_with_sof(0xc2, 0x22);
        assert_eq!(
            parse_jpeg_tile_info(&jpeg).unwrap(),
            JpegTileInfo {
                subsampling: (2, 2)
            }
        );
    }

    #[test]
    fn rejects_decoded_fallback_for_zif_destination() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("fallback.zif");
        let mut encoder = ZifTiffEncoder::new(destination.clone(), Vec2d { x: 1, y: 1 }).unwrap();

        let err = encoder
            .add_tile(Tile {
                position: Vec2d { x: 0, y: 0 },
                image: image::DynamicImage::new_rgba8(1, 1),
                icc_profile: None,
                exif_metadata: None,
            })
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(!destination.with_extension("tiff").exists());
    }

    #[test]
    fn allows_decoded_fallback_for_tiff_destination() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("fallback.tiff");
        let mut encoder = ZifTiffEncoder::new(destination.clone(), Vec2d { x: 1, y: 1 }).unwrap();

        encoder
            .add_tile(Tile {
                position: Vec2d { x: 0, y: 0 },
                image: image::DynamicImage::new_rgba8(1, 1),
                icc_profile: None,
                exif_metadata: None,
            })
            .unwrap();
        encoder.finalize().unwrap();

        assert!(destination.exists());
    }

    fn minimal_jpeg_with_sof(marker: u8, first_component_sampling: u8) -> Vec<u8> {
        vec![
            0xff,
            0xd8,
            0xff,
            marker,
            0x00,
            0x11,
            0x08,
            0x00,
            0x10,
            0x00,
            0x10,
            0x03,
            0x01,
            first_component_sampling,
            0x00,
            0x02,
            0x11,
            0x00,
            0x03,
            0x11,
            0x00,
            0xff,
            0xd9,
        ]
    }
}
