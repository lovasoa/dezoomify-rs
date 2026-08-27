use std::fmt::Debug;

use serde::Deserialize;

use crate::Vec2d;
use crate::core::resolve_relative;
use crate::json_utils::number_or_string;
use url::Url;

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct DziFile {
    #[serde(
        rename = "@Overlap",
        alias = "Overlap",
        deserialize_with = "number_or_string",
        default
    )]
    pub overlap: u32,
    #[serde(
        rename = "@TileSize",
        alias = "TileSize",
        deserialize_with = "number_or_string"
    )]
    pub tile_size: u32,
    #[serde(rename = "@Format", alias = "Format")]
    pub format: String,
    #[serde(rename = "Size")]
    pub size: Size,
    #[serde(rename = "@Url", alias = "Url")]
    pub base_url: Option<String>,
}

impl DziFile {
    pub fn get_size(&self) -> Vec2d {
        Vec2d {
            x: self.size.width,
            y: self.size.height,
        }
    }
    pub fn get_tile_size(&self) -> Vec2d {
        Vec2d::square(self.tile_size)
    }
    pub fn max_level(&self) -> u32 {
        let size = self.get_size();
        log2(size.x.max(size.y))
    }
    pub fn base_url(&self, resource_url: &str) -> String {
        if let Some(s) = &self.base_url {
            let relative_url_str = s.trim_end_matches('/');
            resolve_relative(resource_url, relative_url_str)
        } else {
            implicit_base_url(resource_url)
        }
    }
}

fn implicit_base_url(resource_url: &str) -> String {
    if let Ok(mut url) = Url::parse(resource_url) {
        let path = url.path().to_string();
        let stripped_path = strip_last_path_segment_extension(&path);
        let tile_path = format!("{stripped_path}_files");
        url.set_path(&tile_path);
        url.set_query(None);
        url.set_fragment(None);
        return url.to_string();
    }

    format!("{}_files", strip_last_path_segment_extension(resource_url))
}

fn strip_last_path_segment_extension(path: &str) -> std::borrow::Cow<'_, str> {
    let last_separator = path.rfind(['/', '\\']);
    let last_segment_start = last_separator.map_or(0, |idx| idx + 1);
    let last_segment = &path[last_segment_start..];

    if let Some(dot_pos) = last_segment.rfind('.') {
        std::borrow::Cow::Owned(path[..last_segment_start + dot_pos].to_string())
    } else {
        std::borrow::Cow::Borrowed(path)
    }
}

fn log2(n: u32) -> u32 {
    32 - (n - 1).leading_zeros()
}

#[derive(Debug, Deserialize, PartialEq, Eq, Default)]
pub struct Size {
    #[serde(
        rename = "@Width",
        alias = "Width",
        deserialize_with = "number_or_string",
        default
    )]
    pub width: u32,
    #[serde(
        rename = "@Height",
        alias = "Height",
        deserialize_with = "number_or_string",
        default
    )]
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
    assert_eq!(dzi.get_size(), Vec2d { x: 5393, y: 3852 });
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
    )
    .unwrap();
    assert_eq!(dzi.get_size(), Vec2d { y: 4409, x: 7793 });
    assert_eq!(dzi.get_tile_size(), Vec2d { x: 254, y: 254 });
    assert_eq!(dzi.max_level(), 13);
}

#[test]
fn test_base_url_without_extension_ignores_host_dots() {
    let dzi = DziFile {
        overlap: 0,
        tile_size: 256,
        format: "jpg".to_string(),
        size: Size {
            width: 482_096,
            height: 5550,
        },
        base_url: None,
    };

    assert_eq!(
        dzi.base_url("https://www.bayeuxmuseum.com/datasviewer/manifest"),
        "https://www.bayeuxmuseum.com/datasviewer/manifest_files"
    );
}

#[test]
fn test_base_url_strips_only_last_path_segment_extension() {
    let dzi = DziFile {
        overlap: 0,
        tile_size: 256,
        format: "jpg".to_string(),
        size: Size {
            width: 1,
            height: 1,
        },
        base_url: None,
    };

    assert_eq!(
        dzi.base_url("https://example.com/a.b/image.dzi"),
        "https://example.com/a.b/image_files"
    );
}

#[test]
fn test_base_url_drops_query_and_fragment() {
    let dzi = DziFile {
        overlap: 0,
        tile_size: 256,
        format: "jpg".to_string(),
        size: Size {
            width: 1,
            height: 1,
        },
        base_url: None,
    };

    assert_eq!(
        dzi.base_url("https://host/image.dzi?token=abc#frag"),
        "https://host/image_files"
    );
}
