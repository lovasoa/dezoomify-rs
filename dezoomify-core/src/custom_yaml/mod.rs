//! Pure discovery for explicit `tiles.yaml` layouts.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::Deserialize;

use crate::Vec2d;
use crate::core::discovery::DiscoveryEvent;
use crate::core::{
    CatalogEntry, DiscoveryDiagnostic, DiscoveryError, DiscoveryInput, DiscoveryProgram,
    DiscoverySession, DiscoveryStep, ImageCatalog, ImageDescriptor, KnownTilePlan, LevelDescriptor,
    LevelPlan, PlanError, ProcessingRecipe, ReplayablePlan, Request, ResourceOutcome, ResourceRequest, StableId, TileId, TileRole, TileSpec,
};

mod tile_set;
mod variable;

fn default_headers() -> HashMap<String, String> {
    serde_yaml::from_str(include_str!("../../../src/default_headers.yaml"))
        .expect("bundled default headers must be valid YAML")
}

/// Explicit YAML layout discovery program.
#[derive(Default)]
pub struct CustomDezoomer;

impl DiscoveryProgram for CustomDezoomer {
    fn start(&self, input: &DiscoveryInput) -> Box<dyn DiscoverySession> {
        Box::new(CustomSession {
            uri: input.uri.clone(),
            requested: false,
        })
    }
}

struct CustomSession {
    uri: String,
    requested: bool,
}

impl DiscoverySession for CustomSession {
    fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
        match event {
            DiscoveryEvent::Start if !self.uri.ends_with("tiles.yaml") => Ok(
                DiscoveryStep::Reject(DiscoveryDiagnostic::from("not a tiles.yaml file")),
            ),
            DiscoveryEvent::Start if !self.requested => {
                self.requested = true;
                Ok(DiscoveryStep::Need(ResourceRequest::new(
                    self.uri.clone(),
                )))
            }
            DiscoveryEvent::Resource(ResourceOutcome::Response(response)) => {
                catalog_from_yaml(&response.bytes).map(DiscoveryStep::Complete)
            }
            DiscoveryEvent::Resource(ResourceOutcome::Failure(failure)) => {
                Err(DiscoveryError::Session(failure.message.clone()))
            }
            DiscoveryEvent::Start => Err(DiscoveryError::Session(
                "custom YAML session started twice".into(),
            )),
        }
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

fn catalog_from_yaml(bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let yaml: CustomYamlTiles = serde_yaml::from_slice(bytes)
        .map_err(|error| DiscoveryError::Session(format!("invalid tiles.yaml: {error}")))?;
    let headers: BTreeMap<_, _> = yaml.headers.into_iter().collect();
    let tile_count = yaml
        .tile_set
        .len()
        .map_err(|error| DiscoveryError::Session(format!("invalid tiles.yaml: {error}")))?;
    let size = yaml.width.zip(yaml.height).map(|(x, y)| Vec2d { x, y });
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("custom:image"),
        title: yaml.title,
        format: StableId::new("custom"),
        levels: vec![LevelDescriptor {
            id: StableId::new("custom:level"),
            title: None,
            size,
            tile_size: None,
            scale_factor: None,
            has_overlapping_tiles: false,
            plan: LevelPlan::Known(KnownTilePlan::new(CustomPlan {
                tile_set: yaml.tile_set,
                headers: Arc::new(headers),
                tile_count,
            })),
            warnings: Vec::new(),
        }],
        warnings: Vec::new(),
    })]))
}

#[derive(Clone, Debug)]
struct CustomPlan {
    tile_set: tile_set::TileSet,
    headers: Arc<BTreeMap<String, String>>,
    tile_count: u64,
}

impl ReplayablePlan for CustomPlan {
    fn len(&self) -> u64 {
        self.tile_count
    }

    fn tile(&self, ordinal: u64) -> Result<Option<TileSpec>, PlanError> {
        if ordinal >= self.tile_count {
            return Ok(None);
        }
        let entry = self
            .tile_set
            .tile_at(ordinal)
            .map_err(|error| PlanError::InvalidTile(error.to_string()))?;
        Ok(Some(TileSpec {
            id: TileId::new(StableId::new("custom:level"), ordinal),
            request: Request {
                uri: entry.uri,
                headers: (*self.headers).clone(),
            },
            destination: entry.position,
            expected_size: None,
            processing: ProcessingRecipe::None,
            role: TileRole::Output,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let plan = match &image.levels[0].plan {
            LevelPlan::Known(plan) => plan,
            LevelPlan::Adaptive(_) => panic!("custom YAML has a known plan"),
        };
        assert_eq!(plan.len(), 4);
        let first = plan.tile(0).unwrap().unwrap();
        let last = plan.tile(3).unwrap().unwrap();
        assert_eq!(first.request.uri, "https://example.test/0/0");
        assert_eq!(last.request.uri, "https://example.test/1/1");
        assert_eq!(first, plan.tile(0).unwrap().unwrap());
    }
}
