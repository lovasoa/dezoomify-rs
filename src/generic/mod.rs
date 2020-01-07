use crate::dezoomer::{
    single_level,
    Dezoomer, DezoomerError, DezoomerInput,
    TileFetchResult, TileProvider, TileReference,
    ZoomLevels,
};
use crate::Vec2d;

enum Stage {
    FirstLine { current_x: u32 },
    NextLines { max_x: u32, current_y: u32 },
}

struct ZoomLevel {
    url_template: String,
    stage: Stage,
    tile_size: Option<Vec2d>,
}

impl ZoomLevel {
    fn tile_url_at(&self, x: u32, y: u32) -> String {
        self.url_template
            .replace("{{X}}", &x.to_string())
            .replace("{{Y}}", &y.to_string())
    }
    fn tile_ref_at(&self, x: u32, y: u32) -> TileReference {
        let tile_size = self.tile_size.unwrap_or(Vec2d { x: 0, y: 0 });
        let position = Vec2d { x, y } * tile_size;
        TileReference {
            url: self.tile_url_at(x, y),
            position,
        }
    }
}

impl TileProvider for ZoomLevel {
    fn next_tiles(&mut self, previous: Option<TileFetchResult>) -> Vec<TileReference> {
        match (previous, &self.stage) {
            // First request
            (None, _) => vec![self.tile_ref_at(0, 0)],

            // Advance in the first line
            (
                Some(TileFetchResult { tile_size, successes, count, .. }),
                &Stage::FirstLine { current_x }
            ) => {
                if current_x == 0 { self.tile_size = tile_size; }
                let current_x = current_x + successes as u32;
                if successes == count { // The first line is not over
                    self.stage = Stage::FirstLine { current_x };
                    // We don't want to make too many useless requests,
                    // and we don't want to request tiles one by one either in order to be fast.
                    // At each step, we estimate the total number of tiles in the line as
                    // max(current number of tiles, 4) * 2
                    (current_x..current_x.max(4) * 2)
                        .map(|x| self.tile_ref_at(x, 0))
                        .collect()
                } else { // We had at least one failed tile, the line is over
                    let max_x = current_x - 1;
                    self.stage = Stage::NextLines { max_x, current_y: 1 };
                    (0..=max_x).map(|x| self.tile_ref_at(x, 1)).collect()
                }
            }

            // Advance to next line
            (Some(ref res), &Stage::NextLines { current_y, max_x }) if res.is_success() => {
                let current_y = current_y + 1;
                self.stage = Stage::NextLines { max_x, current_y };
                (0..=max_x)
                    .map(|x| self.tile_ref_at(x, current_y))
                    .collect()
            }

            // End of image
            (Some(_), Stage::NextLines { .. }) => vec![],
        }
    }

    fn name(&self) -> String {
        format!("Generic image with template {}", self.url_template)
    }
}

impl std::fmt::Debug for ZoomLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Generic level")
    }
}

#[derive(Default)]
pub struct GenericDezoomer;

impl Dezoomer for GenericDezoomer {
    fn name(&self) -> &'static str {
        "generic"
    }

    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        self.assert(data.uri.contains("{{X}}"))?;
        let dezoomer = ZoomLevel {
            url_template: data.uri.clone(),
            stage: Stage::FirstLine { current_x: 0 },
            tile_size: None,
        };
        single_level(dezoomer)
    }
}

#[test]
fn test_generic_dezoomer() {
    let uri = "{{X}},{{Y}}".to_string();
    let mut lvl = GenericDezoomer {}
        .zoom_levels(&DezoomerInput {
            uri,
            contents: None,
        })
        .unwrap()
        .into_iter()
        .next()
        .unwrap();

    let existing_tiles = vec!["0,0", "1,0", "2,0", "0,1", "1,1", "2,1"];

    let mut all_tiles = vec![];

    let mut zoom_level_iter = crate::dezoomer::ZoomLevelIter::new(&mut lvl);
    while let Some(tiles) = zoom_level_iter.next() {
        let count = tiles.len() as u64;

        let successes: Vec<_> = tiles
            .into_iter()
            .filter(|t| existing_tiles.contains(&t.url.as_str()))
            .collect();
        zoom_level_iter.set_fetch_result(TileFetchResult {
            count,
            successes: successes.len() as u64,
            tile_size: Some(Vec2d { x: 4, y: 5 }),
        });
        all_tiles.extend(successes);
    };

    assert_eq!(
        all_tiles,
        vec![
            TileReference {
                url: "0,0".into(),
                position: Vec2d { x: 0, y: 0 }
            },
            TileReference {
                url: "1,0".into(),
                position: Vec2d { x: 4, y: 0 }
            },
            TileReference {
                url: "2,0".into(),
                position: Vec2d { x: 8, y: 0 }
            },
            TileReference {
                url: "0,1".into(),
                position: Vec2d { x: 0, y: 5 }
            },
            TileReference {
                url: "1,1".into(),
                position: Vec2d { x: 4, y: 5 }
            },
            TileReference {
                url: "2,1".into(),
                position: Vec2d { x: 8, y: 5 }
            },
        ]
    )
}
