//! Pure NYPL metadata discovery.

use std::collections::HashMap;
use std::sync::Arc;

use regex::Regex;
use serde::Deserialize;

use crate::Vec2d;
use crate::core::discovery::DiscoveryEvent;
use crate::core::tile_plan::RectangularSource;
use crate::core::{
    CatalogEntry, Dezoomer, DezoomerMeta, DiscoveryDiagnostic, DiscoveryError, DiscoveryInput,
    DiscoveryStep, ImageCatalog, ImageDescriptor, KnownTilePlan, LevelDescriptor, LevelPlan,
    ProcessingRecipe, Request, ResourceOutcome, ResourceRequest, StableId,
};
use crate::json_utils::number_or_string;

const VIEW: &str = "https://digitalcollections.nypl.org/items/";
const META: &str = "https://access.nypl.org/image.php/";
const POSTFIX: &str = "/tiles/config.js";

pub struct Nypl {
    input: String,
    requested: bool,
    metadata_uri: Option<String>,
}

impl Dezoomer for Nypl {
    fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
        match event {
            DiscoveryEvent::Start if !self.requested => {
                self.requested = true;
                let meta = if self.input.starts_with(VIEW) {
                    let id = image_id(&self.input).ok_or_else(|| {
                        DiscoveryError::Session(format!(
                            "unable to extract NYPL image ID from {:?}",
                            self.input
                        ))
                    })?;
                    format!("{META}{id}{POSTFIX}")
                } else if self.input.starts_with(META) {
                    self.input.clone()
                } else {
                    return Ok(DiscoveryStep::Reject(DiscoveryDiagnostic::from(
                        "not an NYPL image URL",
                    )));
                };
                self.metadata_uri = Some(meta.clone());
                Ok(DiscoveryStep::Need(ResourceRequest::new(meta)))
            }
            DiscoveryEvent::Resource(ResourceOutcome::Response(response)) => catalog(
                self.metadata_uri.as_deref().unwrap_or(&self.input),
                &response.bytes,
            )
            .map(DiscoveryStep::Complete),
            DiscoveryEvent::Resource(ResourceOutcome::Failure(failure)) => {
                Err(DiscoveryError::Session(failure.message.clone()))
            }
            DiscoveryEvent::Start => {
                Err(DiscoveryError::Session("NYPL session started twice".into()))
            }
        }
    }
}

impl DezoomerMeta for Nypl {
    const NAME: &'static str = "nypl";
    const URL_HINTS: &'static [&'static str] = &["digitalcollections.nypl.org"];

    fn start(input: &DiscoveryInput) -> Self {
        Self {
            input: input.uri.clone(),
            requested: false,
            metadata_uri: None,
        }
    }
}

fn image_id(uri: &str) -> Option<String> {
    Regex::new(r"https://digitalcollections\.nypl\.org/items/([a-f0-9\-]+)")
        .expect("constant NYPL pattern")
        .captures(uri)
        .and_then(|capture| capture.get(1))
        .map(|match_| match_.as_str().to_owned())
}
fn catalog(uri: &str, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    if bytes.is_empty() {
        return Err(DiscoveryError::Session(
            "No metadata found. This image is probably not tiled, and you can download it directly by right-clicking on it from your browser without any external tool.".into(),
        ));
    }
    let root: MetadataRoot = serde_json::from_slice(bytes).map_err(|error| {
        DiscoveryError::Session(format!(
            "Failed to parse NYPL Image meta as json, got content(blank shows the site has no zoom function for this one):\n {}: {error}",
            String::from_utf8_lossy(bytes).chars().take(200).collect::<String>()
        ))
    })?;
    let metadata = root.configs.get("0").ok_or_else(|| {
        DiscoveryError::Session(
            "No metadata found. This image is probably not tiled, and you can download it directly by right-clicking on it from your browser without any external tool.".into(),
        )
    })?.clone();
    let id = uri
        .strip_prefix(META)
        .unwrap_or(uri)
        .trim_end_matches(POSTFIX)
        .to_owned();
    let metadata = Arc::new(metadata);
    let mut levels: Vec<_> = (0..=metadata.level_count())
        .map(|index| {
            let size = Vec2d::from(metadata.size) / 2_u32.pow(metadata.level_count() - index);
            let level_id = StableId::new(format!("nypl:{index}"));
            let source = NyplLevel {
                id: Arc::from(id.as_str()),
                metadata: Arc::clone(&metadata),
                index,
                level_id: level_id.clone(),
            };
            LevelDescriptor {
                id: level_id,
                title: Some(format!("NYPL level {index} ({}×{} pixels)", size.x, size.y)),
                size: Some(size),
                tile_size: Some(Vec2d::square(metadata.tile_size)),
                has_overlapping_tiles: metadata.overlap > 0,
                plan: LevelPlan::Known(KnownTilePlan::rectangular(source)),
                ..Default::default()
            }
        })
        .collect();
    levels.sort_by_key(|level| level.size.map_or(0, Vec2d::area));
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("nypl:image"),
        format: StableId::new("nypl"),
        levels,
        ..Default::default()
    })]))
}

#[derive(Clone, Debug)]
struct NyplLevel {
    id: Arc<str>,
    metadata: Arc<Metadata>,
    index: u32,
    level_id: StableId,
}
impl RectangularSource for NyplLevel {
    fn level_id(&self) -> StableId {
        self.level_id.clone()
    }
    fn image_size(&self) -> Vec2d {
        Vec2d::from(self.metadata.size) / 2_u32.pow(self.metadata.level_count() - self.index)
    }
    fn tile_size(&self) -> Vec2d {
        Vec2d::square(self.metadata.tile_size)
    }
    fn request(&self, cell: Vec2d) -> Request {
        Request::new(format!(
            "{META}{}/tiles/0/{}/{}_{}.{}",
            self.id, self.index, cell.x, cell.y, self.metadata.format
        ))
    }
    fn overlap(&self) -> Vec2d {
        Vec2d::square(self.metadata.overlap)
    }
    fn processing(&self) -> ProcessingRecipe {
        ProcessingRecipe::None
    }
}

#[derive(Debug, Deserialize)]
struct MetadataRoot {
    configs: HashMap<String, Metadata>,
}
#[derive(Clone, Debug, Deserialize)]
struct Metadata {
    size: MetadataSize,
    #[serde(alias = "tilesize", deserialize_with = "number_or_string")]
    tile_size: u32,
    format: String,
    #[serde(default, deserialize_with = "number_or_string")]
    overlap: u32,
}
impl Metadata {
    fn level_count(&self) -> u32 {
        32 - self.size.width.max(self.size.height).leading_zeros()
    }
}
#[derive(Clone, Copy, Debug, Deserialize)]
struct MetadataSize {
    #[serde(deserialize_with = "number_or_string")]
    width: u32,
    #[serde(deserialize_with = "number_or_string")]
    height: u32,
}
impl From<MetadataSize> for Vec2d {
    fn from(size: MetadataSize) -> Self {
        Self {
            x: size.width,
            y: size.height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::TileProgram;
    #[test]
    fn parses_metadata_and_tile_url() {
        let bytes = br#"{"configs":{"0":{"size":{"width":"2422","height":"3000"},"tilesize":"256","overlap":"2","format":"png"}}}"#;
        let catalog =
            catalog("https://access.nypl.org/image.php/a/tiles/config.js", bytes).unwrap();
        let CatalogEntry::Ready(image) = catalog.into_entries().pop().unwrap() else {
            unreachable!()
        };
        assert_eq!(
            image.levels.last().unwrap().size,
            Some(Vec2d { x: 2422, y: 3000 })
        );
        let LevelPlan::Known(plan) = &image.levels.last().unwrap().plan else {
            unreachable!()
        };
        assert_eq!(
            plan.cursor().take_ready(1).unwrap().unwrap()[0].request.uri,
            "https://access.nypl.org/image.php/a/tiles/0/12/0_0.png"
        );
        assert_eq!(image_id("https://digitalcollections.nypl.org/items/a14f3200-fac1-012f-f7a4-58d385a7bbd0#item-data").as_deref(), Some("a14f3200-fac1-012f-f7a4-58d385a7bbd0"));
    }
}
