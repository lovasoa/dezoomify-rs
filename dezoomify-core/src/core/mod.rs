//! Pure values used by the dezooming core.
//!
//! This module deliberately contains no protocol client, runtime, image, or
//! filesystem types.  It describes work for an application to perform.

pub mod adaptive;
pub mod discovery;
pub mod model;
pub mod registry;
pub mod tile_plan;
pub mod uri;

pub use adaptive::TileObservation;
pub use discovery::{
    DiscoveryDiagnostic, DiscoveryError, DiscoveryInput, DiscoveryProgram, DiscoverySession,
    DiscoveryStep, ResourceOutcome, ResourcePurpose, ResourceRequest,
};
#[cfg(test)]
pub use discovery::{RequestId, ResourceResponse};
pub use model::{
    CatalogEntry, DeferredImage, ImageCatalog, ImageDescriptor, LevelDescriptor, ProcessingRecipe,
    Provenance, ProvenanceStep, Request, StableId, TileId, TileRole, TileSpec,
};
pub use registry::{Priority, Registry};
pub use tile_plan::{KnownTilePlan, LevelPlan, PlanError, ReplayablePlan};
pub use uri::resolve_relative;
