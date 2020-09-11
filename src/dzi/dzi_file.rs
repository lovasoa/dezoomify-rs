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
    #[serde(rename = "Url")]
    pub base_url: Option<String>,
}

impl DziFile {
    pub fn get_size(&self) -> Result<Vec2d, DziError> {
        let size = self.sizes.get(0).ok_or(DziError::NoSize)?;
        Ok(Vec2d { x: size.width, y: size.height })
    }
    pub fn get_tile_size(&self) -> Vec2d {
        Vec2d::square(self.tile_size)
    }
    pub fn max_level(&self) -> u32 {
        let size = self.get_size().unwrap();
        log2(size.x.max(size.y))
    }

    /// Compute the base URL for tiles. If a base URL is specified in this structure, remove it
    pub fn take_base_url(&mut self, resource_url: &str) -> String {
        if let Some(mut url_str) = self.base_url.take() {
            if url_str.ends_with('/') { url_str.pop(); }
            url_str
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
