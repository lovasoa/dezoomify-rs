//! Pure NYPL metadata discovery.

use std::collections::HashMap;
use std::sync::Arc;

use regex::Regex;
use serde::Deserialize;

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryError, DiscoveryMatch, Grid, ImageCatalog,
    ImageDescriptor, LevelDescriptor, Request, StableId,
};
use crate::json_utils::number_or_string;

const VIEW: &str = "https://digitalcollections.nypl.org/items/";
const META: &str = "https://access.nypl.org/image.php/";
const POSTFIX: &str = "/tiles/config.js";

pub const SPEC: DezoomerSpec = DezoomerSpec::new(
    "nypl",
    &[
        DiscoveryMatch::UrlPredicate(is_view_url).map_url(metadata_url),
        DiscoveryMatch::Any.extract(catalog),
    ],
)
.recognizing(
    |uri| uri.starts_with(VIEW) || uri.starts_with(META),
    "not an NYPL image URL",
)
.preferring(|uri| uri.contains("digitalcollections.nypl.org"));

fn is_view_url(uri: &str) -> bool {
    uri.starts_with(VIEW)
}

fn metadata_url(input: &str) -> Result<Request, DiscoveryError> {
    let id = image_id(input).ok_or_else(|| {
        DiscoveryError::Session(format!("unable to extract NYPL image ID from {input:?}"))
    })?;
    Ok(Request::new(format!("{META}{id}{POSTFIX}")))
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
        .filter_map(|index| {
            let size = Vec2d::from(metadata.size) / 2_u32.pow(metadata.level_count() - index);
            (size.x > 0 && size.y > 0).then_some((index, size))
        })
        .map(|(index, size)| {
            let id: Arc<str> = Arc::from(id.as_str());
            let format = metadata.format.clone();
            let source = Grid::with_requests(
                format!("nypl:{index}").into(),
                size,
                Vec2d::square(metadata.tile_size),
                Vec2d::square(metadata.overlap),
                move |tile| {
                    let cell: Vec2d = tile.coord.into();
                    Request::new(format!(
                        "{META}{id}/tiles/0/{index}/{}_{}.{format}",
                        cell.x, cell.y
                    ))
                },
            )
            .map_err(|error| DiscoveryError::Session(format!("invalid NYPL grid: {error}")))?;
            Ok(LevelDescriptor::new(source).with_title(Some(format!(
                "NYPL level {index} ({: >5}×{: >5} pixels)",
                size.x, size.y,
            ))))
        })
        .collect::<Result<Vec<_>, DiscoveryError>>()?;
    levels.sort_by_key(|level| level.source.image_size().map_or(0, Vec2d::area));
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("nypl:image"),
        format: StableId::new("nypl"),
        levels,
        ..Default::default()
    })]))
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
    use crate::core::TileSource;
    #[test]
    fn parses_metadata_and_tile_url() {
        let bytes = br#"{"configs":{"0":{"size":{"width":"2422","height":"3000"},"tilesize":"256","overlap":"2","format":"png"}}}"#;
        let catalog =
            catalog("https://access.nypl.org/image.php/a/tiles/config.js", bytes).unwrap();
        let CatalogEntry::Ready(image) = catalog.into_entries().pop().unwrap() else {
            unreachable!()
        };
        assert_eq!(
            image.levels.last().unwrap().source.image_size(),
            Some(Vec2d { x: 2422, y: 3000 })
        );
        let TileSource::Grid(plan) = &image.levels.last().unwrap().source else {
            unreachable!()
        };
        assert_eq!(
            plan.tiles_row_major().next().unwrap().unwrap().request.uri,
            "https://access.nypl.org/image.php/a/tiles/0/12/0_0.png"
        );
        assert_eq!(image_id("https://digitalcollections.nypl.org/items/a14f3200-fac1-012f-f7a4-58d385a7bbd0#item-data").as_deref(), Some("a14f3200-fac1-012f-f7a4-58d385a7bbd0"));
    }
}
