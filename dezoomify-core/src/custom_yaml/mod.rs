//! Pure discovery for explicit `tiles.yaml` layouts.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryError, DiscoveryMatch, ImageCatalog, ImageDescriptor,
    LevelDescriptor, Positioned, ProcessingRecipe, Request, StableId, TileSourceError,
};
use crate::default_headers;

mod tile_set;
mod variable;

pub const SPEC: DezoomerSpec = DezoomerSpec::new("custom", &[DiscoveryMatch::Any.extract(catalog)])
    .recognizing(|uri| uri.ends_with("tiles.yaml"), "not a tiles.yaml file");

fn catalog(_: &str, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    catalog_from_yaml(bytes)
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

fn catalog_from_yaml(bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let yaml: CustomYamlTiles = serde_yaml::from_slice(bytes)
        .map_err(|error| DiscoveryError::Session(format!("invalid tiles.yaml: {error}")))?;
    let headers: BTreeMap<_, _> = yaml.headers.into_iter().collect();
    yaml.tile_set
        .len()
        .map_err(|error| DiscoveryError::Session(format!("invalid tiles.yaml: {error}")))?;
    let size = yaml.width.zip(yaml.height).map(|(x, y)| Vec2d { x, y });
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("custom:image"),
        title: yaml.title,
        format: StableId::new("custom"),
        levels: vec![LevelDescriptor::new(Positioned::from_generator(
            StableId::new("custom:level"),
            size,
            CustomTiles {
                tile_set: yaml.tile_set,
                headers,
            },
        ))],
        ..Default::default()
    })]))
}

#[derive(Clone, Debug)]
struct CustomTiles {
    tile_set: tile_set::TileSet,
    headers: BTreeMap<String, String>,
}

impl crate::core::tile_plan::PositionedGenerator for CustomTiles {
    fn count(&self) -> u64 {
        self.tile_set.len().expect("tile domain was validated")
    }

    fn tile(
        &self,
        ordinal: u64,
    ) -> Result<crate::core::tile_plan::PositionedTile, TileSourceError> {
        let entry = self
            .tile_set
            .tile_at(ordinal)
            .map_err(|error| TileSourceError::InvalidTile(error.to_string()))?;
        Ok(crate::core::tile_plan::PositionedTile {
            request: Request {
                uri: entry.uri,
                headers: self.headers.clone(),
            },
            destination: entry.position,
            processing: ProcessingRecipe::None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{TileId, TileRole, TileSource};

    #[test]
    fn parses_bundled_example_headers() {
        let yaml_path = format!("{}/../tiles.yaml", env!("CARGO_MANIFEST_DIR"));
        let yaml: CustomYamlTiles =
            serde_yaml::from_reader(std::fs::File::open(yaml_path).unwrap()).unwrap();
        assert!(yaml.headers.contains_key("Referer"));
        let catalog = catalog_from_yaml(include_bytes!("../../../tiles.yaml")).unwrap();
        let CatalogEntry::Ready(image) = &catalog.entries()[0] else {
            panic!("custom YAML is immediately ready")
        };
        assert_eq!(image.title.as_deref(), Some("A Palace"));
    }

    #[test]
    fn uses_bundled_default_headers() {
        let yaml: CustomYamlTiles =
            serde_yaml::from_str("url_template: test.com\nvariables: []").unwrap();
        assert!(yaml.headers.contains_key("User-Agent"));
    }

    #[test]
    fn template_plan_is_replayable_without_collecting_tiles() {
        let catalog = catalog_from_yaml(
            br#"
variables:
  - name: x
    from: 0
    to: 1
  - name: y
    from: 0
    to: 1
url_template: "https://example.test/{{x}}/{{y}}"
x_template: x
y_template: y
"#,
        )
        .unwrap();
        let image = match &catalog.entries()[0] {
            CatalogEntry::Ready(image) => image,
            CatalogEntry::Deferred(_) => panic!("custom YAML is immediately ready"),
        };
        let TileSource::Positioned(plan) = &image.levels[0].source else {
            panic!("custom YAML is positioned");
        };
        assert_eq!(plan.count(), 4);
        let tiles: Vec<_> = plan.tiles().collect::<Result<_, _>>().unwrap();
        let first = &tiles[0];
        let last = &tiles[3];
        assert_eq!(first.id, TileId::new("custom:level".into(), 0));
        assert_eq!(first.role, TileRole::Output);
        assert_eq!(first.request.uri, "https://example.test/0/0");
        assert_eq!(last.request.uri, "https://example.test/1/1");
        assert_eq!(first, &plan.tiles().next().unwrap().unwrap());
    }

    #[test]
    fn tile_expression_errors_keep_their_message() {
        // x_template evaluates to a value larger than u32: the error must
        // surface its own message, not a generic geometry-overflow message.
        let catalog = catalog_from_yaml(
            br#"
variables:
  - name: x
    from: 0
    to: 1
url_template: "https://example.test/{{x}}"
x_template: "5000000000 + x"
y_template: y
"#,
        )
        .unwrap();
        let image = match &catalog.entries()[0] {
            CatalogEntry::Ready(image) => image,
            CatalogEntry::Deferred(_) => panic!("custom YAML is immediately ready"),
        };
        let TileSource::Positioned(plan) = &image.levels[0].source else {
            panic!("custom YAML is positioned");
        };
        let error = plan.tiles().next().unwrap().unwrap_err().to_string();
        assert!(
            error.contains("Number too large"),
            "unexpected error message: {error}"
        );
        assert!(
            !error.contains("overflowed u32"),
            "expression errors must not be reported as geometry overflow: {error}"
        );
    }
}
