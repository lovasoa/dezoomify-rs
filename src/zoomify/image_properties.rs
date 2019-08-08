use serde::Deserialize;

use crate::dezoomer::Vec2d;

#[derive(Debug, Deserialize, PartialEq)]
pub struct ImageProperties {
    #[serde(rename = "WIDTH", default)]
    pub width: u32,
    #[serde(rename = "HEIGHT", default)]
    pub height: u32,
    #[serde(rename = "TILESIZE", default)]
    pub tile_size: u32,
    #[serde(rename = "NUMTILES", default)]
    pub num_tiles: u32,
}

impl ImageProperties {
    fn size(&self) -> Vec2d {
        Vec2d { x: self.width, y: self.height }
    }
    fn tile_size(&self) -> Vec2d {
        Vec2d { x: self.tile_size, y: self.tile_size }
    }
    pub fn levels(&self) -> impl Iterator<Item=ZoomLevelInfo> {
        let mut remaining_tiles = self.num_tiles as i64;
        let mut size = self.size();
        let tile_size = self.tile_size();
        std::iter::from_fn(move || {
            if remaining_tiles <= 0 { None } else {
                let tiles_x = (size.x as f32 / tile_size.x as f32).ceil() as u32;
                let tiles_y = (size.y as f32 / tile_size.y as f32).ceil() as u32;
                remaining_tiles -= (tiles_x * tiles_y) as i64;
                let tiles_before = remaining_tiles as u32;
                let lvl = ZoomLevelInfo { tiles_before, tile_size, size };
                size = size / Vec2d { x: 2, y: 2 };
                Some(lvl)
            }
        })
    }
}

pub struct ZoomLevelInfo {
    pub size: Vec2d,
    pub tile_size: Vec2d,
    pub tiles_before: u32,
}

impl ZoomLevelInfo {
    pub fn tile_group(&self, pos: Vec2d) -> u32 {
        let num_tiles_x = (self.size / self.tile_size).x;
        (self.tiles_before + pos.x + pos.y * num_tiles_x) / 256
    }
}

#[test]
fn test_deserialize() {
    let src = r#"
        <IMAGE_PROPERTIES
            WIDTH="4000" HEIGHT="2559"
            NUMTILES="217"
            NUMIMAGES="1"
            VERSION="1.8"
            TILESIZE="256" />"#;
    let props: ImageProperties = serde_xml_rs::from_str(src).unwrap();
    assert_eq!(props.width, 4000);
    assert_eq!(props.height, 2559);
    assert_eq!(props.tile_size, 256);
    assert_eq!(props.num_tiles, 217);
}
