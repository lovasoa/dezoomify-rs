use std::fmt::Debug;

use serde::Deserialize;

use crate::json_utils::number_or_string;
use crate::network::resolve_relative;
use crate::Vec2d;

use super::DziError;

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct DziFile {
    #[serde(rename = "Overlap", deserialize_with = "number_or_string", default)]
    pub overlap: u32,
    #[serde(rename = "TileSize", deserialize_with = "number_or_string")]
    pub tile_size: u32,
    #[serde(rename = "Format")]
    pub format: String,
    #[serde(rename = "Size")]
    pub size: Size,
    #[serde(rename = "Url")]
    pub base_url: Option<String>,
}

impl DziFile {
    pub fn get_size(&self) -> Result<Vec2d, DziError> {
        Ok(Vec2d { x: self.size.width, y: self.size.height })
    }
    pub fn get_tile_size(&self) -> Vec2d {
        Vec2d::square(self.tile_size)
    }
    pub fn max_level(&self) -> u32 {
        let size = self.get_size().unwrap();
        log2(size.x.max(size.y))
    }
    pub fn base_url(&self, resource_url: &str) -> String {
        if let Some(s) = &self.base_url {
            let relative_url_str = s.trim_end_matches('/');
            resolve_relative(resource_url, relative_url_str)
        } else {
            let until_dot = if let Some(dot_pos) = resource_url.rfind('.') {
                &resource_url[0..dot_pos]
            } else { resource_url };
            format!("{}_files", until_dot)
        }
    }
}

fn log2(n: u32) -> u32 {
    32 - (n - 1).leading_zeros()
}

#[derive(Debug, Deserialize, PartialEq, Eq, Default)]
pub struct Size {
    #[serde(rename = "Width", deserialize_with = "number_or_string", default)]
    pub width: u32,
    #[serde(rename = "Height", deserialize_with = "number_or_string", default)]
    pub height: u32,
}

#[test]
fn test_dzi() {
    let dzi: DziFile = serde_xml_rs::from_str(
        r#"
        <Image
            Format="png" Overlap="2" TileSize="256">
            <Size Height="3852" Width="5393"/>
        </Image>"#,
    )
        .unwrap();
    assert_eq!(dzi.get_size().unwrap(), Vec2d { x: 5393, y: 3852 });
    assert_eq!(dzi.get_tile_size(), Vec2d { x: 256, y: 256 });
    assert_eq!(dzi.max_level(), 13);
}

#[test]
fn test_dzi_json() {
    let dzi: DziFile = serde_json::from_str(
        r#"{
            "type":  "image",
            "xmlns": "http://schemas.microsoft.com/deepzoom/2008",
	        "Url":   "http://content.example.net/images/",
            "Format":   "jpg",
            "Overlap":  "1",
            "TileSize": "254",
            "Size": { "Height": "4409", "Width": "7793" }
	    }"#,
    ).unwrap();
    assert_eq!(dzi.get_size().unwrap(), Vec2d { y: 4409, x: 7793 });
    assert_eq!(dzi.get_tile_size(), Vec2d { x: 254, y: 254 });
    assert_eq!(dzi.max_level(), 13);
}
