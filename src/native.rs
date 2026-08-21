//! Native application driver for the pure discovery core.
//!
//! The types in [`dezoomify_core::core`] only describe resources.  This module
//! is the boundary which interprets those descriptions using reqwest or the
//! local filesystem.  Keeping the loop here also lets tests drive discovery
//! with a different resolver without pulling a runtime into the core.

use crate::ZoomError;
use crate::network::{FetchedResource, fetch_resource, request_headers};
use custom_error::custom_error;
use dezoomify_core::core::discovery::{
    DiscoveryError, DiscoveryOperation, ResourceFailure, ResourceNeed, ResourceResponse,
};
use dezoomify_core::core::model::{ImageCatalog, Request};
use dezoomify_core::core::registry::{Registry, RegistryError};

// Errors which can occur while the native application drives discovery.
custom_error! {pub NativeDiscoveryError
    Registry{source: RegistryError} = "invalid discovery registry: {source}",
    Discovery{source: DiscoveryError} = "discovery failed: {source}",
}

/// Operation-scoped native acquisition state.
///
/// A resolver drives discovery operations by acquiring each resource the
/// operation needs through HTTP or the local filesystem.  Identical requests
/// are already deduplicated inside each operation, so the driver needs no
/// shared state beyond the HTTP client.
pub(crate) struct NativeDiscoveryDriver {
    http: reqwest::Client,
}

impl NativeDiscoveryDriver {
    pub(crate) fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    /// Resolve one canonical resource request.
    pub(crate) async fn resolve(&self, request: &Request) -> Result<FetchedResource, ZoomError> {
        let headers = request_headers(request);
        fetch_resource(
            &request.uri,
            &self.http,
            headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )
        .await
    }

    /// Start and drive discovery for an input using the supplied pure registry.
    pub(crate) async fn discover(
        &self,
        registry: &Registry,
        uri: impl Into<String>,
    ) -> Result<ImageCatalog, NativeDiscoveryError> {
        let operation = registry.start(uri.into())?;
        self.drive(operation).await
    }

    /// Drive a previously-created pure discovery operation to completion.
    pub(crate) async fn drive(
        &self,
        mut operation: DiscoveryOperation,
    ) -> Result<ImageCatalog, NativeDiscoveryError> {
        loop {
            let needs = operation.missing_resources()?;
            if operation.is_complete() || needs.is_empty() {
                return operation.finish().map_err(Into::into);
            }
            // Drive again after every supplied resource so a successful,
            // higher-priority candidate can finish before unrelated candidate
            // metadata is acquired.
            self.satisfy(&mut operation, needs[0].clone()).await?;
        }
    }

    async fn satisfy(
        &self,
        operation: &mut DiscoveryOperation,
        need: ResourceNeed,
    ) -> Result<(), DiscoveryError> {
        match self.resolve(&need.request).await {
            Ok(resource) => operation.provide(ResourceResponse {
                id: need.id,
                bytes: resource.bytes,
                content_type: resource.content_type,
            }),
            Err(error) => operation.provide_failure(ResourceFailure {
                id: need.id,
                message: error.to_string(),
            }),
        }
    }
}
