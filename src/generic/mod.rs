use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use crate::Vec2d;
use crate::dezoomer::{
    Dezoomer, DezoomerError, DezoomerInput, Images, TileFetchResult, TileProvider, TileReference,
    single_level,
};

mod dichotomy_2d;

/// A dezoomer that takes an image tile URL template like
/// `http://example.com/image_{{X}}_{{Y}}.jpg`
/// and automatically figures out the dimensions of the image.
#[derive(Default)]
pub struct GenericDezoomer;

impl Dezoomer for GenericDezoomer {
    fn name(&self) -> &'static str {
        "generic"
    }

    fn images(&mut self, data: &DezoomerInput) -> Result<Images, DezoomerError> {
        self.assert(TEMPLATE_RE.is_match(&data.uri))?;
        let dezoomer = ZoomLevel {
            url_template: data.uri.clone(),
            dichotomy: dichotomy_2d::Dichotomy2d::default(),
            last_tile: (0, 0),
            done: HashSet::new(),
            tile_size: None,
            image_size: None,
        };
        Ok(single_level(dezoomer).into())
    }
}

static TEMPLATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?xi)
    \{\{
        (?P<dimension>x|y)
        (?::0(?P<zeroes>\d+))?
     \}\}
    ",
    )
    .unwrap()
});

struct ZoomLevel {
    url_template: String,
    dichotomy: dichotomy_2d::Dichotomy2d,
    last_tile: (u32, u32),
    tile_size: Option<Vec2d>,
    image_size: Option<Vec2d>,
    done: HashSet<(u32, u32)>,
}

impl ZoomLevel {
    fn tile_url_at(&self, x: u32, y: u32) -> String {
        TEMPLATE_RE
            .replace_all(&self.url_template, |caps: &regex::Captures| {
                let dimension = caps
                    .name("dimension")
                    .expect("missing dimension")
                    .as_str()
                    .chars()
                    .next()
                    .expect("empty dim")
                    .to_ascii_lowercase();
                let num = match dimension {
                    'x' => x,
                    'y' => y,
                    _ => unreachable!("The dimension is either x or y"),
                };
                let padding: usize = caps
                    .name("zeroes")
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(0);
                format!("{num:0padding$}")
            })
            .to_string()
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
        if let Some(p) = previous {
            self.tile_size = self.tile_size.or(p.tile_size);
            if let Some((x, y)) = self.dichotomy.next(p.is_success()) {
                self.last_tile = (x, y);
                self.done.insert((x, y));
                vec![self.tile_ref_at(x, y)]
            } else if !self.done.is_empty() {
                let last_tile_pos = Vec2d {
                    x: self.last_tile.0,
                    y: self.last_tile.1,
                };
                self.image_size = self.tile_size.map(|s| s * last_tile_pos + s);
                let all_tiles = (0..=last_tile_pos.y)
                    .flat_map(|y| (0..=last_tile_pos.x).map(move |x| (x, y)))
                    .filter(|pos| !self.done.contains(pos))
                    .map(|(x, y)| self.tile_ref_at(x, y))
                    .collect();
                self.done.clear();
                all_tiles
            } else {
                vec![]
            }
        } else {
            vec![self.tile_ref_at(self.last_tile.0, self.last_tile.1)]
        }
    }
    fn name(&self) -> String {
        format!("Generic image with template {}", self.url_template)
    }
    fn size_hint(&self) -> Option<Vec2d> {
        self.image_size
    }
}

impl std::fmt::Debug for ZoomLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Generic level")
    }
}

#[test]
fn test_generic_dezoomer() {
    use crate::dezoomer::PageContents;
    use crate::dezoomer::test_utils::expect_single_resolved;
    use std::collections::HashSet;
    let uri = "{{X}},{{Y}}".to_string();
    let images = GenericDezoomer {}
        .images(&DezoomerInput {
            uri,
            contents: PageContents::Unknown,
        })
        .unwrap();
    let image = expect_single_resolved(images);
    let mut lvl = image.into_zoom_levels().into_iter().next().unwrap();

    let existing_tiles = ["0,0", "1,0", "2,0", "0,1", "1,1", "2,1"];

    let mut all_tiles = HashSet::new();

    let mut zoom_level_iter = crate::dezoomer::ZoomLevelIter::new(&mut lvl);
    let mut tries = 0;
    while let Some(tiles) = zoom_level_iter.next_tile_references() {
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
        tries += 1;
        assert!(tries <= 10);
    }

    let expected: HashSet<TileReference> = vec![
        TileReference {
            url: "0,0".into(),
            position: Vec2d { x: 0, y: 0 },
        },
        TileReference {
            url: "1,0".into(),
            position: Vec2d { x: 4, y: 0 },
        },
        TileReference {
            url: "2,0".into(),
            position: Vec2d { x: 8, y: 0 },
        },
        TileReference {
            url: "0,1".into(),
            position: Vec2d { x: 0, y: 5 },
        },
        TileReference {
            url: "1,1".into(),
            position: Vec2d { x: 4, y: 5 },
        },
        TileReference {
            url: "2,1".into(),
            position: Vec2d { x: 8, y: 5 },
        },
    ]
    .into_iter()
    .collect();
    assert_eq!(all_tiles, expected);
}

#[test]
fn test_url_templating() {
    let url_template = "http://x.com/{{x:05}}_{{y}}".to_string();
    let lvl: ZoomLevel = ZoomLevel {
        url_template,
        dichotomy: dichotomy_2d::Dichotomy2d::default(),
        last_tile: (0, 0),
        tile_size: None,
        image_size: None,
        done: HashSet::default(),
    };
    assert_eq!(lvl.tile_url_at(10, 11), "http://x.com/00010_11");
    assert_eq!(lvl.tile_url_at(123, 1), "http://x.com/00123_1");
}
