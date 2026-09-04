//! IIIF output: writes tile images and an `info.json` pyramid description.

use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;

use image::codecs::jpeg::JpegEncoder;
use log::debug;

use crate::encoder::retiler::{Retiler, TileSaver};
use crate::errors::image_error_to_io_error;
use crate::tile::Tile;
use crate::{Vec2d, ZoomError};
use dezoomify_core::iiif::tile_info;

use super::{Encoder, SourceLevel};

pub struct IiifEncoder {
    retiler: Retiler<IIIFTileSaver>,
    root_path: PathBuf,
    direct_tile_saver: Arc<IIIFTileSaver>,
    direct_levels: Vec<SourceLevel>,
    current_level: Option<SourceLevel>,
}

impl IiifEncoder {
    pub fn new(destination: PathBuf, size: Vec2d, quality: u8) -> Result<Self, ZoomError> {
        let _ = std::fs::remove_file(&destination);
        debug!("Creating IIIF directory at {}", destination.display());
        std::fs::create_dir(&destination)?;
        let tile_saver = Arc::new(IIIFTileSaver {
            root_path: destination.clone(),
            quality,
        });
        let tile_size = Vec2d::square(512);
        Ok(IiifEncoder {
            retiler: Retiler::new(size, tile_size, Arc::clone(&tile_saver), 1),
            root_path: destination,
            direct_tile_saver: tile_saver,
            direct_levels: Vec::new(),
            current_level: None,
        })
    }
}

impl Encoder for IiifEncoder {
    fn begin_level(&mut self, level: SourceLevel) -> io::Result<()> {
        self.current_level = Some(level);
        self.direct_levels.push(level);
        Ok(())
    }

    fn add_tile(&mut self, tile: Tile) -> io::Result<()> {
        if let Some(level) = self.current_level {
            self.direct_tile_saver
                .save_tile_at_scale(level.scale_factor, level.size, &tile)
        } else {
            self.retiler.add_tile(&tile)
        }
    }

    fn finalize(&mut self) -> io::Result<()> {
        if self.direct_levels.is_empty() {
            self.retiler.finalize();
        }
        let scale_factors = if self.direct_levels.is_empty() {
            (0..self.retiler.level_count())
                .map(|n| 2u32.pow(n))
                .collect::<Vec<_>>()
        } else {
            self.direct_levels
                .iter()
                .map(|level| level.scale_factor)
                .collect::<Vec<_>>()
        };
        let tile_size = self
            .direct_levels
            .iter()
            .find_map(|level| level.tile_size)
            .unwrap_or(self.retiler.tile_size);
        let image_info = tile_info::ImageInfo {
            context: Some("http://iiif.io/api/image/3/context.json".to_string()),
            iiif_type: Some("ImageService3".to_string()),
            protocol: Some("http://iiif.io/api/image".to_string()),
            profile: Some(tile_info::Profile::Reference("level0".to_string())),
            id: Some(".".to_string()),
            width: self.size().x,
            height: self.size().y,
            qualities: Some(vec!["default".into()]),
            formats: Some(vec!["jpg".into()]),
            tiles: Some(vec![tile_info::TileInfo {
                width: tile_size.x,
                height: Some(tile_size.y),
                scale_factors,
            }]),
            ..Default::default()
        };
        let info_json_str = serde_json::to_string(&image_info)?;
        let info_json_path = self.root_path.join("info.json");
        let viewer_path = self.root_path.join("viewer.html");
        debug!("Writing iiif metadata to {}", info_json_path.display());
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(info_json_path)?
            .write_all(info_json_str.as_bytes())?;

        debug!("Writing viewer page to {}", viewer_path.display());
        let viewer_buf = include_str!("./viewer_files/viewer.html")
            .replace(
                "/*DEZOOMIFY_SEADRAGON*/",
                include_str!("./viewer_files/openseadragon.min.js"),
            )
            .replace("{/*DEZOOMIFY_TILE_SOURCE*/}", &info_json_str);
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(viewer_path)?
            .write_all(viewer_buf.as_bytes())?;
        Ok(())
    }

    fn size(&self) -> Vec2d {
        self.retiler.size()
    }
}

struct IIIFTileSaver {
    root_path: PathBuf,
    quality: u8,
}

impl IIIFTileSaver {
    fn save_tile_at_scale(
        &self,
        scale_factor: u32,
        image_size: Vec2d,
        tile: &Tile,
    ) -> io::Result<()> {
        let scale = Vec2d::square(scale_factor);
        let full_position = tile.position.checked_mul(scale).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "IIIF tile position overflow")
        })?;
        let full_extent = tile
            .size()
            .checked_mul(scale)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "IIIF tile size overflow"))?;
        let full_size = full_position
            .checked_add(full_extent)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "IIIF tile extent overflow"))?
            .min(image_size)
            - full_position;
        self.save_tile_region(full_position, full_size, tile.size(), tile)
    }

    fn save_tile_region(
        &self,
        full_position: Vec2d,
        full_size: Vec2d,
        tile_size: Vec2d,
        tile: &Tile,
    ) -> io::Result<()> {
        let region = format!(
            "{},{},{},{}",
            full_position.x, full_position.y, full_size.x, full_size.y
        );
        let tile_size_str = format!("{},{}", tile_size.x, tile_size.y);
        let rotation = "0";
        let filename = "default.jpg";
        let mut image_dir_path = self.root_path.clone();
        image_dir_path.push(region);
        image_dir_path.push(tile_size_str);
        image_dir_path.push(rotation);
        let image_path = image_dir_path.join(filename);
        debug!("Writing tile to {}", image_path.display());
        std::fs::create_dir_all(&image_dir_path)?;
        let file = &mut BufWriter::new(File::create(&image_path)?);
        let jpeg_writer = JpegEncoder::new_with_quality(file, self.quality);
        tile.image
            .write_with_encoder(jpeg_writer)
            .map_err(image_error_to_io_error)
    }
}

impl TileSaver for IIIFTileSaver {
    fn save_tile(&self, size: Vec2d, tile: Tile) -> io::Result<()> {
        self.save_tile_region(tile.position, size, tile.size(), &tile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_direct_source_pyramid_levels() {
        let temp_dir = tempfile::tempdir().unwrap();
        let destination = temp_dir.path().join("image.iiif");
        let full_size = Vec2d { x: 4, y: 4 };
        let tile_size = Vec2d { x: 2, y: 2 };
        let mut encoder = IiifEncoder::new(destination.clone(), full_size, 90).unwrap();

        encoder
            .begin_level(SourceLevel {
                index: 0,
                size: full_size,
                scale_factor: 1,
                tile_size: Some(tile_size),
                has_overlapping_tiles: false,
            })
            .unwrap();
        encoder
            .add_tile(Tile::empty(Vec2d::default(), tile_size))
            .unwrap();

        encoder
            .begin_level(SourceLevel {
                index: 1,
                size: full_size,
                scale_factor: 2,
                tile_size: Some(tile_size),
                has_overlapping_tiles: false,
            })
            .unwrap();
        encoder
            .add_tile(Tile::empty(Vec2d::default(), tile_size))
            .unwrap();
        encoder.finalize().unwrap();

        assert!(destination.join("0,0,2,2/2,2/0/default.jpg").is_file());
        assert!(destination.join("0,0,4,4/2,2/0/default.jpg").is_file());
        assert!(destination.join("viewer.html").is_file());

        let info: serde_json::Value =
            serde_json::from_slice(&std::fs::read(destination.join("info.json")).unwrap()).unwrap();
        assert_eq!(info["width"], 4);
        assert_eq!(info["height"], 4);
        assert_eq!(info["tiles"][0]["width"], 2);
        assert_eq!(info["tiles"][0]["height"], 2);
        assert_eq!(info["tiles"][0]["scaleFactors"], serde_json::json!([1, 2]));
    }

    #[test]
    fn clips_scaled_source_tiles_to_the_full_image_size() {
        let temp_dir = tempfile::tempdir().unwrap();
        let destination = temp_dir.path().join("odd.iiif");
        let full_size = Vec2d { x: 1001, y: 1001 };
        let mut encoder = IiifEncoder::new(destination.clone(), full_size, 90).unwrap();

        encoder
            .begin_level(SourceLevel {
                index: 0,
                size: full_size,
                scale_factor: 2,
                tile_size: Some(Vec2d { x: 512, y: 512 }),
                has_overlapping_tiles: false,
            })
            .unwrap();
        encoder
            .add_tile(Tile::empty(Vec2d::default(), Vec2d { x: 501, y: 501 }))
            .unwrap();
        encoder.finalize().unwrap();

        assert!(
            destination
                .join("0,0,1001,1001/501,501/0/default.jpg")
                .is_file()
        );
        assert!(
            !destination
                .join("0,0,1002,1002/501,501/0/default.jpg")
                .exists()
        );
    }
}
