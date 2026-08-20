use std::fmt;

use super::tile_plan::KnownTilePlan;

/// A stable identifier assigned by one discovery or planning operation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StableId(String);

impl StableId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for StableId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for StableId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for StableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A two-dimensional coordinate in an image or tile grid.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Point {
    pub x: u32,
    pub y: u32,
}

impl Point {
    #[must_use]
    pub const fn new(x: u32, y: u32) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self {
            x: self.x.saturating_sub(other.x),
            y: self.y.saturating_sub(other.y),
        }
    }
}

/// Pixel dimensions.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

impl Dimensions {
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    #[must_use]
    pub const fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// A rectangular image region.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Region {
    pub origin: Point,
    pub size: Dimensions,
}

impl Region {
    #[must_use]
    pub const fn new(origin: Point, size: Dimensions) -> Self {
        Self { origin, size }
    }
}

/// A portable requirement attached to a resource request.
///
/// The application decides how to implement these requirements.  In
/// particular, this type does not contain a reqwest header map.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RequestRequirement {
    Header { name: String, value: String },
    AcceptContentType(String),
    Method(String),
}

/// A description of a resource request, not an instruction to perform I/O.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestSpec {
    pub uri: String,
    pub requirements: Vec<RequestRequirement>,
}

impl RequestSpec {
    #[must_use]
    pub fn new(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            requirements: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_requirements(
        uri: impl Into<String>,
        requirements: impl IntoIterator<Item = RequestRequirement>,
    ) -> Self {
        Self {
            uri: uri.into(),
            requirements: requirements.into_iter().collect(),
        }
    }
}

/// A named operation that the application may apply to downloaded bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ProcessingRecipe {
    None,
    GoogleArtsDecrypt,
}

/// Distinguishes discovery probes from tiles which belong to the output.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TileRole {
    Probe,
    Output,
}

/// A deterministic tile identity within a level.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TileId {
    pub level: StableId,
    pub role: TileRole,
    pub ordinal: u64,
}

impl TileId {
    #[must_use]
    pub const fn new(level: StableId, role: TileRole, ordinal: u64) -> Self {
        Self {
            level,
            role,
            ordinal,
        }
    }
}

/// A complete description of one tile, ready for an application to acquire.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TileSpec {
    pub id: TileId,
    pub request: RequestSpec,
    pub source_region: Region,
    pub destination_region: Region,
    pub expected_size: Option<Dimensions>,
    pub processing: ProcessingRecipe,
    pub role: TileRole,
}

/// A provenance step recorded while recognizing or adapting an image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvenanceStep {
    pub id: StableId,
    pub description: String,
}

impl ProvenanceStep {
    #[must_use]
    pub fn new(id: impl Into<StableId>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
        }
    }
}

/// Ordered, deterministic provenance attached to a catalog item.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Provenance {
    pub steps: Vec<ProvenanceStep>,
}

impl Provenance {
    #[must_use]
    pub fn new(steps: impl IntoIterator<Item = ProvenanceStep>) -> Self {
        Self {
            steps: steps.into_iter().collect(),
        }
    }
}

/// A tile program descriptor.  Adaptive execution state belongs to a separate
/// operation-owned value; this descriptor records that a level is adaptive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TileProgram {
    Known(KnownTilePlan),
    Adaptive { id: StableId, description: String },
}

/// Immutable description of a resolution level.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LevelDescriptor {
    pub id: StableId,
    pub title: Option<String>,
    pub dimensions: Option<Dimensions>,
    pub tile_size: Option<Dimensions>,
    pub scale_factor: Option<u32>,
    pub has_overlapping_tiles: bool,
    pub program: TileProgram,
    pub provenance: Provenance,
    pub warnings: Vec<String>,
}

/// Immutable image descriptor with normalized levels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageDescriptor {
    pub id: StableId,
    pub title: Option<String>,
    pub dimensions: Option<Dimensions>,
    pub format: Option<String>,
    pub levels: Vec<LevelDescriptor>,
    pub provenance: Provenance,
    pub warnings: Vec<String>,
}

/// A logical image whose metadata has not yet been supplied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeferredImage {
    pub id: StableId,
    pub uri: String,
    pub title: Option<String>,
    pub provenance: Provenance,
    pub warnings: Vec<String>,
}

/// One catalog entry, either resolved or deferred for later discovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogEntry {
    Resolved(ImageDescriptor),
    Deferred(DeferredImage),
}

/// Immutable ordered catalog produced by discovery.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImageCatalog {
    entries: Vec<CatalogEntry>,
}

impl ImageCatalog {
    #[must_use]
    pub fn new(entries: impl IntoIterator<Item = CatalogEntry>) -> Self {
        Self {
            entries: entries.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn into_entries(self) -> Vec<CatalogEntry> {
        self.entries
    }
}
