use serde::Deserialize;

use crate::Vec2d;

#[derive(Debug, Deserialize, PartialEq)]
pub struct ImageInfo {
    #[serde(rename = "@id")]
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub tiles: Option<Vec<TileInfo>>,
}

impl ImageInfo {
    pub fn size(&self) -> Vec2d {
        Vec2d {
            x: self.width,
            y: self.height,
        }
    }
}
#[derive(Debug, Deserialize, PartialEq)]
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
