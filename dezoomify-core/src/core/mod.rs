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

pub use adaptive::{
    AdaptiveError, TileObservation, TileProgram, TileProgramError,
};
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
pub use processing::ProcessingError;
pub use registry::{Priority, Registry};
pub use tile_plan::{KnownPlanCursor, KnownTilePlan, LevelPlan, PlanError, ReplayablePlan};
pub use uri::resolve_relative;
