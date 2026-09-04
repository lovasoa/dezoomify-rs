//! Neutral values shared by discovery and tile planning.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::sync::Arc;

use crate::Vec2d;

use super::tile_plan::TileSource;

/// A deterministic identifier within one discovery/planning operation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StableId(Arc<str>);

impl StableId {
    #[must_use]
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for StableId {
    fn from(value: &str) -> Self {
        Self::new(Arc::<str>::from(value))
    }
}

impl From<String> for StableId {
    fn from(value: String) -> Self {
        Self::new(Arc::<str>::from(value))
    }
}

impl fmt::Display for StableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One portable resource description, used for both metadata and tiles.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Request {
    pub uri: String,
    pub headers: BTreeMap<String, String>,
}

impl Request {
    #[must_use]
    pub fn new(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }
}

/// A byte-processing operation applied to a fetched tile payload before it
/// is decoded as an image.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ProcessingRecipe {
    None,
    /// Strips Google Arts & Culture tile encryption (see
    /// `google_arts_and_culture::decryption`).
    GoogleArtsDecrypt,
}

/// How an acquired tile participates in adaptive probing and final output.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TileRole {
    Output,
    /// A probe which must not be added to the output canvas.
    Probe,
    /// A successful probe is output; a missing probe is not an output failure.
    ProbeAndOutput,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TileId {
    pub level: StableId,
    pub ordinal: u64,
}

impl TileId {
    #[must_use]
    pub const fn new(level: StableId, ordinal: u64) -> Self {
        Self { level, ordinal }
    }
}

/// A logical tile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TileSpec {
    pub id: TileId,
    pub request: Request,
    /// Top-left output position. The extent is deliberately optional because
    /// probe and custom-layout tiles may only reveal it after decoding.
    pub destination: Vec2d,
    pub expected_size: Option<Vec2d>,
    pub processing: ProcessingRecipe,
    pub role: TileRole,
}

#[derive(Clone, Debug)]
pub struct LevelDescriptor {
    pub title: Option<String>,
    pub scale_factor: Option<u32>,
    pub source: TileSource,
    pub warnings: Vec<String>,
}

impl LevelDescriptor {
    #[must_use]
    pub fn new(source: impl Into<TileSource>) -> Self {
        Self {
            title: None,
            scale_factor: None,
            source: source.into(),
            warnings: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_title(mut self, title: Option<String>) -> Self {
        self.title = title;
        self
    }

    #[must_use]
    pub const fn with_scale_factor(mut self, scale_factor: Option<u32>) -> Self {
        self.scale_factor = scale_factor;
        self
    }

    #[must_use]
    pub fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        self.warnings = warnings;
        self
    }

    #[must_use]
    pub fn id(&self) -> &StableId {
        self.source.id()
    }

    /// Human-readable label for interactive pickers.
    ///
    /// Shows the level title (or stable id as a fallback) followed by the
    /// image size, tile size and tile count whenever they are known.
    #[must_use]
    pub fn display_label(&self) -> String {
        let label = self.title.clone().unwrap_or_else(|| self.id().to_string());
        let size = self.source.image_size();
        let count = self.source.count();
        if size.is_none() && count.is_none() {
            return label;
        }
        let mut out = String::with_capacity(label.len() + 40);
        let _ = write!(out, "{label} (");
        let mut sep = "";
        if let Some(Vec2d { x, y }) = size {
            let _ = write!(out, "{x: >5} x {y: >5} pixels");
            sep = ",";
        }
        if let Some(count) = count {
            let _ = write!(out, "{sep}{count: >4} tiles");
        }
        let _ = write!(out, ")");
        out
    }
}

#[derive(Clone, Debug)]
pub struct ImageDescriptor {
    pub id: StableId,
    pub title: Option<String>,
    pub format: StableId,
    pub levels: Vec<LevelDescriptor>,
    pub warnings: Vec<String>,
}

impl Default for ImageDescriptor {
    fn default() -> Self {
        Self {
            id: StableId::new(""),
            title: None,
            format: StableId::new(""),
            levels: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeferredImage {
    pub id: StableId,
    pub uri: String,
    pub title: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum CatalogEntry {
    Ready(ImageDescriptor),
    Deferred(DeferredImage),
}

#[derive(Clone, Debug, Default)]
pub struct ImageCatalog(pub Vec<CatalogEntry>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogError {
    DuplicateEntryId(StableId),
    DuplicateLevelId {
        image_id: StableId,
        level_id: StableId,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateEntryId(id) => write!(f, "duplicate catalog entry id: {id}"),
            Self::DuplicateLevelId { image_id, level_id } => {
                write!(f, "duplicate level id {level_id} in image {image_id}")
            }
        }
    }
}

impl std::error::Error for CatalogError {}

impl ImageCatalog {
    #[must_use]
    pub fn new(entries: impl IntoIterator<Item = CatalogEntry>) -> Self {
        Self(entries.into_iter().collect())
    }

    #[must_use]
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn into_entries(self) -> Vec<CatalogEntry> {
        self.0
    }

    /// Enforce deterministic catalog ordering and stable identifier uniqueness.
    pub fn normalize(mut self) -> Result<Self, CatalogError> {
        let mut entry_ids = BTreeSet::new();
        for entry in &mut self.0 {
            let id = match entry {
                CatalogEntry::Ready(image) => {
                    let mut level_ids = BTreeSet::new();
                    for level in &image.levels {
                        if !level_ids.insert(level.id().clone()) {
                            return Err(CatalogError::DuplicateLevelId {
                                image_id: image.id.clone(),
                                level_id: level.id().clone(),
                            });
                        }
                    }
                    if image
                        .levels
                        .iter()
                        .all(|level| level.source.image_size().is_some())
                    {
                        image.levels.sort_by_key(|level| {
                            level.source.image_size().expect("all sizes checked").area()
                        });
                    }
                    image.id.clone()
                }
                CatalogEntry::Deferred(image) => image.id.clone(),
            };
            if !entry_ids.insert(id.clone()) {
                return Err(CatalogError::DuplicateEntryId(id));
            }
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Grid, GridRequests, GridTile};

    #[derive(Debug)]
    struct TestSource;

    impl GridRequests for TestSource {
        fn request(&self, tile: GridTile) -> Request {
            Request::new(format!("memory://{}/{}", tile.coord.column, tile.coord.row))
        }
    }

    fn level(id: &str, size: u32) -> LevelDescriptor {
        LevelDescriptor::new(
            Grid::new(
                id.into(),
                Vec2d::square(size),
                Vec2d::square(1),
                Vec2d::default(),
                TestSource,
            )
            .unwrap(),
        )
    }

    #[test]
    fn display_label_includes_geometry_and_tile_count() {
        let mut level = level("gap:0", 100);
        level.source = Grid::new(
            "level".into(),
            Vec2d::square(100),
            Vec2d::square(100),
            Vec2d::default(),
            TestSource,
        )
        .unwrap()
        .into();
        let label = level.display_label();
        assert_eq!(label, "level (  100 x   100 pixels,   1 tiles)");
        level.title = Some("Krpano Cube forward".into());
        assert!(level.display_label().starts_with("Krpano Cube forward ("));
    }

    #[test]
    fn normalization_orders_levels_and_rejects_duplicate_stable_ids() {
        let catalog = ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
            id: StableId::new("image"),
            title: None,
            format: StableId::new("test"),
            levels: vec![level("large", 300), level("small", 100)],
            warnings: Vec::new(),
        })])
        .normalize()
        .unwrap();
        let CatalogEntry::Ready(image) = &catalog.entries()[0] else {
            unreachable!()
        };
        assert_eq!(image.levels[0].id().as_str(), "small");

        let duplicate = ImageCatalog::new([
            CatalogEntry::Deferred(DeferredImage {
                id: StableId::new("same"),
                uri: "memory://one".into(),
                title: None,
                warnings: Vec::new(),
            }),
            CatalogEntry::Deferred(DeferredImage {
                id: StableId::new("same"),
                uri: "memory://two".into(),
                title: None,
                warnings: Vec::new(),
            }),
        ]);
        assert!(matches!(
            duplicate.normalize(),
            Err(CatalogError::DuplicateEntryId(id)) if id.as_str() == "same"
        ));
    }
}
