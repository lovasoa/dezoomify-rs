use serde::Deserialize;

use log::{info, warn};
use crate::dezoomer::Vec2d;
use std::convert::TryInto;

#[derive(Debug, Deserialize, PartialEq, Eq)]
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
        Vec2d {
            x: self.width,
            y: self.height,
        }
    }
    fn tile_size(&self) -> Vec2d {
        Vec2d {
            x: self.tile_size,
            y: self.tile_size,
        }
    }

    pub fn levels(&self) -> Vec<ZoomLevelInfo> {
        // Reimplementation of the algorithm of zoomify.js
        let tile_size = self.tile_size();
        let mut width = self.width as f64;
        let mut height = self.height as f64;
        let mut level_tiles = Vec::new();
        let mut tiles_before = Vec::new();
        let tile_width = tile_size.x as f64;
        let tile_height = tile_size.y as f64;
        while width > tile_width || height > tile_height {
            let tiles = (width / tile_width).ceil() * (height / tile_height).ceil();
            tiles_before.push(tiles as u32);
            level_tiles.push(ZoomLevelInfo {
                size: Vec2d { x: width as u32, y: height as u32 },
                tile_size,
                tiles_before: 0, // Will be replaced in the end
            });
            width /= 2.;
            height /= 2.;
        }
        let computed_tile_count = tiles_before.iter().sum::<u32>();
        if computed_tile_count != self.num_tiles {
            info!("The computed number of tiles ({}) does not match \
            the number of tiles specified in ImageProperties.xml ({}). \
            Trying the second computation method..."
                  , computed_tile_count, self.num_tiles);
            level_tiles.clear();
            tiles_before.clear();
            let mut size = self.size();
            let mut level_size_ratio = Vec2d { x: 2, y: 2 };
            loop {
                let size_in_tiles = size.ceil_div(tile_size);
                tiles_before.push(size_in_tiles.area().try_into().unwrap());
                level_tiles.push(ZoomLevelInfo { size, tile_size, tiles_before: 0 });
                if size.x <= tile_size.x && size.y <= tile_size.y { break }
                size = self.size() / level_size_ratio;
                if size.x % 2 != 0 { size.x += 1 }
                if size.y % 2 != 0 { size.y += 1 }
                level_size_ratio = level_size_ratio * Vec2d { x: 2, y: 2 };
            }
        }
        if log::log_enabled!(log::Level::Warn) {
            let computed_tile_count = tiles_before.iter().sum::<u32>();
            if computed_tile_count != self.num_tiles {
                warn!("The computed number of tiles ({}) does not match \
                        the number of tiles specified in ImageProperties.xml ({})"
                      , computed_tile_count, self.num_tiles);
            }
        }
        level_tiles.reverse();
        let mut total_tiles_before = 0;
        let levels_before = level_tiles.iter_mut().zip(tiles_before.iter().rev());
        for (level, &before) in levels_before {
            level.tiles_before = total_tiles_before;
            total_tiles_before += before
        }
        level_tiles
    }
}

#[derive(Debug, PartialEq, Eq)]
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

#[test]
fn test_real_num_tiles() {
    // An image with 3 levels: 10x5 6x2 and 2x2
    let props = ImageProperties {
        width: 10,
        height: 5,
        tile_size: 3,
        num_tiles: 4 * 2,
    };
    let tile_size = Vec2d { x: 3, y: 3 };
    assert_eq!(
        props.levels(),
        vec![
            ZoomLevelInfo { size: Vec2d { x: 2, y: 2 }, tile_size, tiles_before: 0 },
            ZoomLevelInfo { size: Vec2d { x: 6, y: 2 }, tile_size, tiles_before: 1 },
            ZoomLevelInfo { size: Vec2d { x: 10, y: 5 }, tile_size, tiles_before: 3 },
        ]);
}

#[test]
fn test_levels_recount() {
    // See: https://github.com/lovasoa/dezoomify-rs/issues/35
    // The official implementation returns
    // https://gist.github.com/lovasoa/a1442d684a6cabb6e7fe790e4f765f02
    // get_tile_counts(2052, 3185, 256, 256, 117)
    // {
    //   "level_tile_count_y": [1,2,4,7,13],
    //   "level_tile_count_x": [1,1,3,5,9],
    //   "level_tile_count": [1,2,12,35,117],
    //   "level_widths": [128,256,514,1026,2052],
    //   "level_heights": [200,398,796,1592,3185]
    // }
    let img_prop = ImageProperties {
        width: 2052,
        height: 3185,
        tile_size: 256,
        num_tiles: 117,
    };
    let actual_levels: Vec<ZoomLevelInfo> = img_prop.levels();
    let expected_levels: Vec<ZoomLevelInfo> = vec![
        ZoomLevelInfo {
            size: Vec2d { x: 128, y: 200 },
            tile_size: Vec2d { x: 256, y: 256 },
            tiles_before: 0,
        },
        ZoomLevelInfo {
            size: Vec2d { x: 256, y: 398 },
            tile_size: Vec2d { x: 256, y: 256 },
            tiles_before: 1,
        },
        ZoomLevelInfo {
            size: Vec2d { x: 514, y: 796 },
            tile_size: Vec2d { x: 256, y: 256 },
            tiles_before: 1 + 2,
        },
        ZoomLevelInfo {
            size: Vec2d { x: 1026, y: 1592 },
            tile_size: Vec2d { x: 256, y: 256 },
            tiles_before: 1 + 2 + 12,
        },
        ZoomLevelInfo {
            size: Vec2d { x: 2052, y: 3185 },
            tile_size: Vec2d { x: 256, y: 256 },
            tiles_before: 1 + 2 + 12 + 35,
        },
    ];
    assert_eq!(actual_levels, expected_levels);
}