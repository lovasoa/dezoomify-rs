use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;

use image::ImageError;

use crate::{Vec2d, ZoomError};
use crate::iiif::tile_info;
use crate::tile::Tile;

use super::Encoder;

pub struct IiifEncoder {
    root_path: PathBuf,
    size: Vec2d,
    tile_size: Option<Vec2d>,
}

impl IiifEncoder {
    pub fn new(destination: PathBuf, size: Vec2d) -> Result<Self, ZoomError> {
        let _ = std::fs::remove_file(&destination);
        std::fs::create_dir(&destination)?;
        Ok(IiifEncoder {
            root_path: destination,
            size,
            tile_size: None,
        })
    }
}

impl Encoder for IiifEncoder {
    fn add_tile(&mut self, tile: Tile) -> io::Result<()> {
        let tile_size = tile.size();
        self.tile_size = Some(self.tile_size.unwrap_or(tile_size).max(tile_size));
        let region = format!("{},{},{},{}",
                             tile.position.x, tile.position.y,
                             tile_size.x, tile_size.y);
        let size = "max";
        let rotation = "0";
        let filename = "default.jpg";
        let image_dir_path = self.root_path
            .join(region)
            .join(size)
            .join(rotation);
        let image_path = image_dir_path.join(filename);
        std::fs::create_dir_all(&image_dir_path)?;
        tile.image.save(image_path).map_err(image_error_to_io_error)
    }

    fn finalize(self: &mut Self) -> io::Result<()> {
        let tile_size = self.tile_size
            .ok_or_else(|| make_io_err("No tile"))?;
        let image_info = tile_info::ImageInfo {
            id: Some(self.root_path.to_string_lossy().to_string()),
            width: self.size.x,
            height: self.size.y,
            qualities: Some(vec!["default".into()]),
            formats: Some(vec!["jpg".into()]),
            tiles: Some(vec![
                tile_info::TileInfo {
                    width: tile_size.x,
                    height: Some(tile_size.y),
                    scale_factors: vec![1],
                }
            ]),
            ..Default::default()
        };
        let info_json_path = self.root_path.join("info.json");
        let info_json_file = OpenOptions::new().write(true).create(true).open(info_json_path)?;
        serde_json::to_writer(info_json_file, &image_info)?;
        Ok(())
    }

    fn size(&self) -> Vec2d {
        self.size
    }
}

fn image_error_to_io_error(err: ImageError) -> io::Error {
    match err {
        ImageError::IoError(e) => e,
        e => make_io_err(e)
    }
}

fn make_io_err<E>(e: E) -> io::Error
    where E: Into<Box<dyn std::error::Error + Send + Sync>> {
    io::Error::new(io::ErrorKind::Other, e)
}