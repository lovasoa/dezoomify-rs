//! Neutral values shared by discovery and tile planning.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::sync::Arc;

use crate::Vec2d;

use super::tile_plan::LevelPlan;

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
    pub id: StableId,
    pub title: Option<String>,
    pub size: Option<Vec2d>,
    pub tile_size: Option<Vec2d>,
    pub scale_factor: Option<u32>,
    pub has_overlapping_tiles: bool,
    pub plan: LevelPlan,
    pub warnings: Vec<String>,
}

impl Default for LevelDescriptor {
    fn default() -> Self {
        Self {
            id: StableId::new(""),
            title: None,
            size: None,
            tile_size: None,
            scale_factor: None,
            has_overlapping_tiles: false,
            plan: LevelPlan::default(),
            warnings: Vec::new(),
        }
    }
}

impl LevelDescriptor {
    /// Human-readable label for interactive pickers.
    ///
    /// Shows the level title (or stable id as a fallback) followed by the
    /// image size, tile size and tile count whenever they are known.
    #[must_use]
    pub fn display_label(&self) -> String {
        let label = self.title.clone().unwrap_or_else(|| self.id.to_string());
        let has_size = self.size.is_some();
        let has_plan = matches!(&self.plan, LevelPlan::Known(_));
        if !has_size && !has_plan {
            return label;
        }
        let mut out = String::with_capacity(label.len() + 40);
        let _ = write!(out, "{label} (");
        let mut sep = "";
        if let Some(Vec2d { x, y }) = self.size {
            let _ = write!(out, "{x: >5} x {y} pixels");
            sep = ",";
        }
        if let LevelPlan::Known(plan) = &self.plan {
            let _ = write!(out, "{sep}{: >4} tiles", plan.len());
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
                        if !level_ids.insert(level.id.clone()) {
                            return Err(CatalogError::DuplicateLevelId {
                                image_id: image.id.clone(),
                                level_id: level.id.clone(),
                            });
                        }
                    }
                    if image.levels.iter().all(|level| level.size.is_some()) {
                        image
                            .levels
                            .sort_by_key(|level| level.size.expect("all sizes checked").area());
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
    use crate::core::{KnownTilePlan, LevelPlan};

    #[derive(Debug)]
    struct TestSource {
        size: Vec2d,
    }

    impl crate::core::tile_plan::RectangularSource for TestSource {
        fn level_id(&self) -> crate::core::StableId {
            "level".into()
        }
        fn image_size(&self) -> Vec2d {
            self.size
        }
        fn tile_size(&self) -> Vec2d {
            Vec2d::square(1)
        }
        fn request(&self, p: Vec2d) -> Request {
            Request::new(format!("memory://{}/{}", p.x, p.y))
        }
    }

    fn level(id: &str, size: u32) -> LevelDescriptor {
        LevelDescriptor {
            id: StableId::new(id),
            size: Some(Vec2d::square(size)),
            plan: LevelPlan::Known(KnownTilePlan::rectangular(TestSource {
                size: Vec2d::square(size),
            })),
            ..Default::default()
        }
    }

    #[test]
    fn display_label_includes_geometry_and_tile_count() {
        let mut level = level("gap:0", 100);
        level.tile_size = Some(Vec2d::square(100));
        level.plan = LevelPlan::Known(KnownTilePlan::rectangular(TestSource {
            size: Vec2d::square(1),
        }));
        let label = level.display_label();
        assert!(label.contains("100 x 100 pixels"));
        assert!(label.contains("1 tiles"));
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
        assert_eq!(image.levels[0].id.as_str(), "small");

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
