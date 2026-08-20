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

pub use adaptive::{AdaptiveError, AdaptiveProgram, ObservationResult, TileObservation};
pub use discovery::{
    Delegation, DiscoveryDiagnostic, DiscoveryError, DiscoveryInput, DiscoveryLimits,
    DiscoveryOperation, DiscoveryRule, DiscoveryRuleSession, FormatHandler, FormatSession, Profile,
    ProvenanceEvent, RequestId, RequestRequirements, ResourceFailure, ResourceNeed,
    ResourceOutcome, ResourcePurpose, ResourceRequest, ResourceResponse, SessionStep,
};
pub use model::{
    CatalogEntry, DeferredImage, Dimensions, ImageCatalog, ImageDescriptor, LevelDescriptor, Point,
    ProcessingRecipe, Provenance, ProvenanceStep, Region, RequestRequirement, RequestSpec,
    StableId, TileId, TileProgram, TileRole, TileSpec,
};
pub use registry::{Priority, RegistrationId, Registry, RegistryError};
pub use tile_plan::{KnownPlanCursor, KnownTilePlan, PlanError};
pub use uri::resolve_relative;
