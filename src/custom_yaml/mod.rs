use std::collections::HashMap;

use serde::Deserialize;

use crate::network::default_headers;
use crate::dezoomer::*;
use crate::TileReference;

mod tile_set;
mod variable;

/// A dezoomer that takes a yaml file indicating the tile layout
#[derive(Default)]
pub struct CustomDezoomer;

impl Dezoomer for CustomDezoomer {
    fn name(&self) -> &'static str {
        "custom"
    }

    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        self.assert(data.uri.ends_with("tiles.yaml"))?;
        let contents = data.with_contents()?.contents;
        let dezoomer: CustomYamlTiles =
            serde_yaml::from_slice(contents).map_err(DezoomerError::wrap)?;
        single_level(dezoomer)
    }
}


#[derive(Deserialize)]
struct CustomYamlTiles {
    #[serde(flatten)]
    tile_set: tile_set::TileSet,
    #[serde(default = "default_headers")]
    headers: HashMap<String, String>,
    title: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

impl std::fmt::Debug for CustomYamlTiles {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Custom tiles")
    }
}

impl TileProvider for CustomYamlTiles {
    fn next_tiles(&mut self, previous: Option<TileFetchResult>) -> Vec<TileReference> {
        if previous.is_some() {
            return vec![];
        }
        let tiles_result: Result<Vec<_>, _> = self.tile_set.into_iter().collect();
        match tiles_result {
            Ok(tiles) => tiles,
            Err(err) => {
                log::error!("Invalid tiles.yaml file: {}\n", err);
                vec![]
            }
        }
    }

    fn title(&self) -> Option<String> { self.title.clone() }

    fn size_hint(&self) -> Option<Vec2d> {
        if let (Some(x), Some(y)) = (self.width, self.height) {
            Some(Vec2d { x, y })
        } else { None }
    }

    fn http_headers(&self) -> HashMap<String, String> {
        self.headers.clone()
    }
}

#[test]
fn test_can_parse_example() {
    use std::fs::File;

    let yaml_path = format!("{}/tiles.yaml", env!("CARGO_MANIFEST_DIR"));
    let file = File::open(yaml_path).unwrap();
    let conf: CustomYamlTiles = serde_yaml::from_reader(file).unwrap();
    assert!(
        conf.http_headers().contains_key("Referer"),
        "There should be a referer in the example"
    );
}

#[test]
fn test_has_default_user_agent() {
    let conf: CustomYamlTiles =
        serde_yaml::from_str("url_template: test.com\nvariables: []").unwrap();
    assert!(
        conf.http_headers().contains_key("User-Agent"),
        "There should be a user agent"
    );
}
