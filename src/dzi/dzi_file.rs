use serde::Deserialize;

use crate::Vec2d;

use super::DziError;

#[derive(Debug, Deserialize, PartialEq)]
pub struct DziFile {
    #[serde(rename = "Overlap", default)]
    pub overlap: u32,
    #[serde(rename = "TileSize", default)]
    pub tile_size: u32,
    #[serde(rename = "Format", default)]
    pub format: String,
    #[serde(rename = "Size", default)]
    pub sizes: Vec<Size>,
}

impl DziFile {
    pub fn get_size(&self) -> Result<Vec2d, DziError> {
        self.sizes
            .iter()
            .next()
            .map(|s| Vec2d {
                x: s.width,
                y: s.height,
            })
            .ok_or(DziError::NoSize)
    }
    pub fn get_tile_size(&self) -> Vec2d {
        Vec2d::square(self.tile_size)
    }
    pub fn max_level(&self) -> u32 {
        let size = self.get_size().unwrap();
        log2(size.x.max(size.y))
    }
}

fn log2(n: u32) -> u32 {
    32 - (n - 1).leading_zeros()
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct Size {
    #[serde(rename = "Width", default)]
    pub width: u32,
    #[serde(rename = "Height", default)]
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
fn test_dzi_with_doctype() {
    // See https://github.com/lovasoa/dezoomify-rs/issues/45
    let dzi: DziFile = serde_xml_rs::from_str(
        r#"<?xml version="1.0" encoding="utf-8"?>
        <Image TileSize="256" Overlap="0" Format="jpg" xmlns="http://schemas.microsoft.com/deepzoom/2008">
        <Size Width="6261" Height="6047" />
        </Image>"#,
    ).unwrap();
    assert_eq!(dzi.get_size().unwrap(), Vec2d { x: 6261, y: 6047 });
    assert_eq!(dzi.get_tile_size(), Vec2d { x: 256, y: 256 });
    assert_eq!(dzi.max_level(), 13);
}
