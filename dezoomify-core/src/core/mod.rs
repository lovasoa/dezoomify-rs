//! Pure values used by the dezooming core.
//!
//! This module deliberately contains no protocol client, runtime, image, or
//! filesystem types.  It describes work for an application to perform.

pub mod adaptive;
pub mod discovery;
pub mod model;
pub mod processing;
pub mod registry;
pub mod tile_plan;
pub mod uri;

pub use adaptive::{DiscoverableGrid, DiscoverableStep, ObservationResult, ProbeContinuation};
pub use discovery::{
    DezoomerSpec, DiscoveryContext, DiscoveryError, DiscoveryFailure, DiscoveryInput,
    DiscoveryMatch, DiscoveryResource, DiscoveryRoute, DiscoveryStep,
};
#[cfg(test)]
pub use discovery::{RequestId, ResourceResponse};
pub use model::{
    CatalogEntry, DeferredImage, ImageCatalog, ImageDescriptor, LevelDescriptor, ProcessingRecipe,
    Request, StableId, TileId, TileRole, TileSpec,
};
pub use processing::ProcessingError;
pub use registry::{Registry, default_registry, registry_for};
pub use tile_plan::{
    Grid, GridCoord, GridRequests, GridTile, Positioned, PositionedTile, TileSource,
    TileSourceError,
};
pub use uri::resolve_relative;
