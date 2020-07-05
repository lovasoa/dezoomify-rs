use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::PathBuf;

use image::ImageError;
use log::debug;

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
        let size = format!("{},{}", tile_size.x, tile_size.y);
        let rotation = "0";
        let filename = "default.jpg";
        let image_dir_path = self.root_path
            .join(region)
            .join(size)
            .join(rotation);
        let image_path = image_dir_path.join(filename);
        debug!("Writing tile to {:?}", image_path);
        std::fs::create_dir_all(&image_dir_path)?;
        tile.image.save(image_path).map_err(image_error_to_io_error)
    }

    fn finalize(self: &mut Self) -> io::Result<()> {
        let tile_size = self.tile_size
            .ok_or_else(|| make_io_err("No tile"))?;
        let image_info = tile_info::ImageInfo {
            context: Some("http://iiif.io/api/image/3/context.json".to_string()),
            iiif_type: Some("ImageService3".to_string()),
            protocol: Some("http://iiif.io/api/image".to_string()),
            profile: Some(tile_info::Profile::Reference("level0".to_string())),
            id: Some(".".to_string()),
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
        let info_json_str = serde_json::to_string(&image_info)?;
        let info_json_path = self.root_path.join("info.json");
        let viewer_path = self.root_path.join("viewer.html");
        debug!("Writing iiif metadata to {:?}", info_json_path);
        OpenOptions::new().write(true).create(true)
            .open(info_json_path)?
            .write_all(info_json_str.as_bytes())?;

        debug!("Writing viewer page to {:?}", viewer_path);
        let viewer_buf = include_str!("./viewer_files/viewer.html")
            .replace("/*DEZOOMIFY_SEADRAGON*/", include_str!("./viewer_files/openseadragon.min.js"))
            .replace("{/*DEZOOMIFY_TILE_SOURCE*/}", &info_json_str);
        OpenOptions::new().write(true).create(true)
            .open(viewer_path)?
            .write_all(viewer_buf.as_bytes())?;
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