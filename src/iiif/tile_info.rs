use serde::Deserialize;

use crate::Vec2d;

#[derive(Debug, Deserialize, PartialEq)]
pub struct ImageInfo {
    #[serde(rename = "@id")]
    pub id: Option<String>,
    pub width: u32,
    pub height: u32,

    qualities: Option<Vec<String>>,

    #[serde(alias = "preferredFormats")]
    formats: Option<Vec<String>>,

    // Used in IIIF version 2 :
    tiles: Option<Vec<TileInfo>>,

    // Used in IIIF version 1 :
    scale_factors: Option<Vec<u32>>,
    tile_width: Option<u32>,
    tile_height: Option<u32>,
}

// Image qualities, from least favorite to favorite
static QUALITY_ORDER: [&'static str; 5] = ["bitonal", "gray", "color", "default", "native"];
// Image formats, from least favorite to favorite
static FORMAT_ORDER: [&'static str; 7] = ["gif", "bmp", "tif", "png", "jpg", "jpeg", "webp"];


impl ImageInfo {
    pub fn size(&self) -> Vec2d {
        Vec2d {
            x: self.width,
            y: self.height,
        }
    }

    pub fn best_quality(&self) -> &str {
        self.qualities.iter().flat_map(|v| v.iter())
            .max_by_key(|&s| QUALITY_ORDER.iter().position(|&x| x == s))
            .map(|s| s.as_str())
            .unwrap_or("default")
    }

    pub fn best_format(&self) -> &str {
        self.formats.iter().flat_map(|v| v.iter())
            .max_by_key(|&s| FORMAT_ORDER.iter().position(|&x| x == s))
            .map(|s| s.as_str())
            .unwrap_or("jpg")
    }

    pub fn tiles(&self) -> Vec<TileInfo> {
        self.tiles.as_ref()
            .and_then(|v|
                if v.is_empty() {
                    None
                } else {
                    Some(v.to_vec())
                })
            .unwrap_or_else(|| {
                let mut info = TileInfo::default();
                if let Some(width) = self.tile_width {
                    info.width = width
                }
                if let Some(height) = self.tile_height {
                    info.height = Some(height)
                }
                if let Some(scale_factors) = &self.scale_factors {
                    info.scale_factors = scale_factors.clone()
                }
                vec![info]
            })
    }
}
#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct TileInfo {
    pub width: u32,
    pub height: Option<u32>,
    #[serde(rename = "scaleFactors")]
    pub scale_factors: Vec<u32>,
}

impl Default for TileInfo {
    fn default() -> Self {
        TileInfo {
            width: 512,
            height: None,
            scale_factors: vec![1],
        }
    }
}

#[test]
fn test_deserialisation() {
    let _: ImageInfo = serde_json::from_str(
        r#"{
      "@context" : "http://iiif.io/api/image/2/context.json",
      "@id" : "http://www.example.org/image-service/abcd1234/1E34750D-38DB-4825-A38A-B60A345E591C",
      "protocol" : "http://iiif.io/api/image",
      "width" : 6000,
      "height" : 4000,
      "sizes" : [
        {"width" : 150, "height" : 100},
        {"width" : 600, "height" : 400},
        {"width" : 3000, "height": 2000}
      ],
      "tiles": [
        {"width" : 512, "scaleFactors" : [1,2,4,8,16]}
      ],
      "profile" : [ "http://iiif.io/api/image/2/level2.json" ]
    }"#,
    )
    .unwrap();
}
