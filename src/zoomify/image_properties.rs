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
        let mut remaining_tiles = i64::from(self.num_tiles);
        let mut size = self.size();
        let tile_size = self.tile_size();
        std::iter::from_fn(move || {
            if remaining_tiles <= 0 { None } else {
                let Vec2d { x: tiles_x, y: tiles_y } = size.ceil_div(tile_size);
                remaining_tiles -= i64::from(tiles_x) * i64::from(tiles_y);
                let tiles_before = remaining_tiles as u32;
                let lvl = ZoomLevelInfo { tiles_before, tile_size, size };
                size = size.ceil_div(Vec2d { x: 2, y: 2 });
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
        let num_tiles_x = (self.size.ceil_div(self.tile_size)).x;
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
