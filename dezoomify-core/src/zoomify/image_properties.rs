use serde::Deserialize;

use crate::Vec2d;

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct ImageProperties {
    #[serde(rename = "@WIDTH", default)]
    pub width: u32,
    #[serde(rename = "@HEIGHT", default)]
    pub height: u32,
    #[serde(rename = "@TILESIZE", default)]
    pub tile_size: u32,
    #[serde(rename = "@NUMTILES", default)]
    pub num_tiles: u32,
}

impl ImageProperties {
    pub(crate) fn size(&self) -> Vec2d {
        Vec2d {
            x: self.width,
            y: self.height,
        }
    }
    pub(crate) fn tile_size(&self) -> Vec2d {
        Vec2d {
            x: self.tile_size,
            y: self.tile_size,
        }
    }

    pub(crate) fn is_full_resolution_only(&self) -> bool {
        let tile_size = self.tile_size();
        if tile_size.x == 0 || tile_size.y == 0 {
            return false;
        }
        let full_resolution_tiles = self.size().ceil_div(tile_size).area();
        if u64::from(self.num_tiles) != full_resolution_tiles {
            return false;
        }
        let mut divisor = 1_u64;
        let mut pyramid_tiles = 0_u64;
        while u64::from(self.width) > u64::from(tile_size.x) * divisor
            || u64::from(self.height) > u64::from(tile_size.y) * divisor
        {
            let tiles_x = u64::from(self.width).div_ceil(u64::from(tile_size.x) * divisor);
            let tiles_y = u64::from(self.height).div_ceil(u64::from(tile_size.y) * divisor);
            pyramid_tiles += tiles_x * tiles_y;
            divisor = divisor.saturating_mul(2);
        }
        pyramid_tiles != u64::from(self.num_tiles)
    }

    #[cfg(test)]
    pub fn levels(&self) -> Vec<ZoomLevelInfo> {
        self.levels_with_warnings().0
    }

    /// Computes the pyramid levels and reports metadata inconsistencies.
    ///
    /// The warning is returned as data so the application can decide how to
    /// present it; parsing a Zoomify document must not have logging side
    /// effects.
    pub fn levels_with_warnings(&self) -> (Vec<ZoomLevelInfo>, Vec<String>) {
        // Reimplementation of the algorithm of zoomify.js
        let tile_size = self.tile_size();
        if tile_size.x == 0 || tile_size.y == 0 {
            return (
                Vec::new(),
                vec!["Zoomify tile size must be greater than zero".into()],
            );
        }
        let mut level_divisor = 1_u64;
        let mut level_tiles = Vec::new();
        let mut tiles_before = Vec::new();
        let mut warnings = Vec::new();
        let full_resolution_only = self.is_full_resolution_only();
        while u64::from(self.width) > u64::from(tile_size.x) * level_divisor
            || u64::from(self.height) > u64::from(tile_size.y) * level_divisor
        {
            let tiles_x = u64::from(self.width).div_ceil(u64::from(tile_size.x) * level_divisor);
            let tiles_y = u64::from(self.height).div_ceil(u64::from(tile_size.y) * level_divisor);
            let tiles = tiles_x * tiles_y;
            tiles_before.push(u32::try_from(tiles).unwrap_or(u32::MAX));
            level_tiles.push(ZoomLevelInfo {
                size: Vec2d {
                    x: u32::try_from(u64::from(self.width) / level_divisor)
                        .expect("a divided u32 always fits in a u32"),
                    y: u32::try_from(u64::from(self.height) / level_divisor)
                        .expect("a divided u32 always fits in a u32"),
                },
                tile_size,
                tiles_before: 0, // Will be replaced in the end
            });
            level_divisor *= 2;
        }
        let computed_tile_count = tiles_before
            .iter()
            .map(|&count| u64::from(count))
            .sum::<u64>();
        if computed_tile_count != u64::from(self.num_tiles) {
            level_tiles.clear();
            tiles_before.clear();
            let mut size = self.size();
            let mut divisor = 1_u64;
            loop {
                let size_in_tiles = size.ceil_div(tile_size);
                tiles_before.push(u32::try_from(size_in_tiles.area()).unwrap_or(u32::MAX));
                level_tiles.push(ZoomLevelInfo {
                    size,
                    tile_size,
                    tiles_before: 0,
                });
                if size.x <= tile_size.x && size.y <= tile_size.y {
                    break;
                }
                divisor = divisor.saturating_mul(2);
                let raw_x = (u64::from(self.width) / divisor).max(1);
                let raw_y = (u64::from(self.height) / divisor).max(1);
                let mut next_x = u32::try_from(raw_x).unwrap_or(u32::MAX);
                let mut next_y = u32::try_from(raw_y).unwrap_or(u32::MAX);
                if !next_x.is_multiple_of(2) && (next_x > tile_size.x || tile_size.x > 1) {
                    next_x = next_x.saturating_add(1);
                }
                if !next_y.is_multiple_of(2) && (next_y > tile_size.y || tile_size.y > 1) {
                    next_y = next_y.saturating_add(1);
                }
                size = Vec2d {
                    x: next_x,
                    y: next_y,
                };
            }
        }
        let computed_tile_count = tiles_before
            .iter()
            .map(|&count| u64::from(count))
            .sum::<u64>();
        if computed_tile_count != u64::from(self.num_tiles) && !full_resolution_only {
            warnings.push(format!(
                "Zoomify tile count mismatch: computed {computed_tile_count}, metadata declares {}",
                self.num_tiles
            ));
        }
        level_tiles.reverse();
        let mut total_tiles_before = 0_u32;
        let levels_before = level_tiles.iter_mut().zip(tiles_before.iter().rev());
        for (level, &before) in levels_before {
            level.tiles_before = total_tiles_before;
            total_tiles_before = total_tiles_before.saturating_add(before);
        }
        (level_tiles, warnings)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ZoomLevelInfo {
    pub size: Vec2d,
    pub tile_size: Vec2d,
    pub tiles_before: u32,
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
            ZoomLevelInfo {
                size: Vec2d { x: 2, y: 2 },
                tile_size,
                tiles_before: 0
            },
            ZoomLevelInfo {
                size: Vec2d { x: 6, y: 2 },
                tile_size,
                tiles_before: 1
            },
            ZoomLevelInfo {
                size: Vec2d { x: 10, y: 5 },
                tile_size,
                tiles_before: 3
            },
        ]
    );
}

#[test]
fn fractional_dimensions_keep_the_next_level() {
    let tile_size = Vec2d::square(256);
    let props = ImageProperties {
        width: 200,
        height: 513,
        tile_size: 256,
        num_tiles: 5,
    };

    assert_eq!(
        props.levels(),
        vec![
            ZoomLevelInfo {
                size: Vec2d { x: 100, y: 256 },
                tile_size,
                tiles_before: 0,
            },
            ZoomLevelInfo {
                size: Vec2d { x: 200, y: 513 },
                tile_size,
                tiles_before: 2,
            },
        ]
    );
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

#[test]
fn malformed_tile_counts_do_not_overflow_level_reconstruction() {
    let props = ImageProperties {
        width: u32::MAX,
        height: u32::MAX,
        tile_size: 1,
        num_tiles: 0,
    };
    let result = std::panic::catch_unwind(|| props.levels_with_warnings());
    assert!(result.is_ok());
}

#[test]
fn zero_tile_size_is_reported_without_dividing_by_zero() {
    let props = ImageProperties {
        width: 100,
        height: 100,
        tile_size: 0,
        num_tiles: 0,
    };
    let (levels, warnings) = props.levels_with_warnings();
    assert!(levels.is_empty());
    assert_eq!(warnings, ["Zoomify tile size must be greater than zero"]);
}
