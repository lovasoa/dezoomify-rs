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
use crate::iiif::tile_info;
use crate::tile::Tile;
use crate::{Vec2d, ZoomError};

use super::Encoder;

pub struct IiifEncoder {
    retiler: Retiler<IIIFTileSaver>,
    root_path: PathBuf,
}

impl IiifEncoder {
    pub fn new(destination: PathBuf, size: Vec2d, quality: u8) -> Result<Self, ZoomError> {
        let _ = std::fs::remove_file(&destination);
        debug!("Creating IIIF  directory at {:?}", &destination);
        std::fs::create_dir(&destination)?;
        let tile_saver = IIIFTileSaver {
            root_path: destination.clone(),
            quality,
        };
        let tile_size = Vec2d::square(512);
        Ok(IiifEncoder {
            retiler: Retiler::new(size, tile_size, Arc::new(tile_saver), 1),
            root_path: destination,
        })
    }
}

impl Encoder for IiifEncoder {
    fn add_tile(&mut self, tile: Tile) -> io::Result<()> {
        self.retiler.add_tile(&tile)
    }

    fn finalize(&mut self) -> io::Result<()> {
        self.retiler.finalize();
        let scale_factors = (0..self.retiler.level_count())
            .map(|n| 2u32.pow(n))
            .collect::<Vec<_>>();
        let tile_size = self.retiler.tile_size;
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
        debug!("Writing iiif metadata to {:?}", info_json_path);
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(info_json_path)?
            .write_all(info_json_str.as_bytes())?;

        debug!("Writing viewer page to {:?}", viewer_path);
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

impl TileSaver for IIIFTileSaver {
    fn save_tile(&self, size: Vec2d, tile: Tile) -> io::Result<()> {
        let tile_size = tile.size();
        let region = format!(
            "{},{},{},{}",
            tile.position.x, tile.position.y, size.x, size.y
        );
        let tile_size_str = format!("{},{}", tile_size.x, tile_size.y);
        let rotation = "0";
        let filename = "default.jpg";
        let mut image_dir_path = self.root_path.clone();
        image_dir_path.push(region);
        image_dir_path.push(tile_size_str);
        image_dir_path.push(rotation);
        let image_path = image_dir_path.join(filename);
        debug!("Writing tile to {:?}", image_path);
        std::fs::create_dir_all(&image_dir_path)?;
        let file = &mut BufWriter::new(File::create(&image_path)?);
        let jpeg_writer = JpegEncoder::new_with_quality(file, self.quality);
        tile.image
            .write_with_encoder(jpeg_writer)
            .map_err(image_error_to_io_error)
    }
}
