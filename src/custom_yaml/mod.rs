use std::collections::HashMap;
use std::error::Error;

use serde::Deserialize;

use crate::default_headers;
use crate::dezoomer::{Dezoomer, DezoomerError, DezoomerInput, single_level, TileProvider, ZoomLevels};
use crate::TileReference;

mod tile_set;
mod variable;

#[derive(Deserialize, Debug)]
struct CustomYamlTiles {
    #[serde(flatten)]
    tile_set: tile_set::TileSet,
    #[serde(default = "default_headers")]
    headers: HashMap<String, String>,
}

impl TileProvider for CustomYamlTiles {
    fn tiles(&self) -> Vec<Result<TileReference, Box<dyn Error>>> {
        self.tile_set.into_iter()
            .map(|r| r.map_err(|e| e.into()))
            .collect()
    }

    fn http_headers(&self) -> HashMap<String, String> {
        self.headers.clone()
    }
}

pub fn dezoom_fn(data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
    DEZOOMER.assert(data.uri.ends_with("tiles.yaml"))?;
    let contents = data.with_contents()?.contents;
    let dezoomer: CustomYamlTiles = serde_yaml::from_slice(&contents)
        .map_err(DezoomerError::wrap)?;
    single_level(dezoomer)
}

pub const DEZOOMER: Dezoomer = Dezoomer {
    name: "custom",
    dezoom_fn,
};

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::{CustomYamlTiles, TileProvider};

    #[test]
    fn test_can_parse_example() {
        let yaml_path = format!("{}/tiles.yaml", env!("CARGO_MANIFEST_DIR"));
        let file = File::open(yaml_path).unwrap();
        let conf: CustomYamlTiles = serde_yaml::from_reader(file).unwrap();
        assert!(conf.http_headers().contains_key("Referer"), "There should be a referer in the example");
    }

    #[test]
    fn test_has_default_user_agent() {
        let conf: CustomYamlTiles = serde_yaml::from_str("url_template: test.com\nvariables: []").unwrap();
        assert!(conf.http_headers().contains_key("User-Agent"), "There should be a user agent");
    }
}