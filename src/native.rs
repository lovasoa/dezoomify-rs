//! Native application driver for the pure discovery core.
//!
//! The types in [`dezoomify_core::core`] only describe resources.  This module
//! is the boundary which interprets those descriptions using reqwest or the
//! local filesystem.  Keeping the loop here also lets tests drive discovery
//! with a different resolver without pulling a runtime into the core.

use std::collections::HashSet;

use crate::ZoomError;
use crate::network::{FetchedResource, effective_request_headers, fetch_resource};
use custom_error::custom_error;
use dezoomify_core::core::discovery::{
    DiscoveryError, DiscoveryOperation, ResourceFailure, ResourceNeed, ResourceResponse,
};
use dezoomify_core::core::model::{ImageCatalog, Request};
use dezoomify_core::core::registry::Registry;

// Errors which can occur while the native application drives discovery.
custom_error! {pub NativeDiscoveryError
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
    user_header_names: HashSet<String>,
}

impl NativeDiscoveryDriver {
    pub(crate) fn with_user_headers(
        http: reqwest::Client,
        user_header_names: HashSet<String>,
    ) -> Self {
        Self {
            http,
            user_header_names,
        }
    }

    /// Resolve one canonical resource request.
    pub(crate) async fn resolve(&self, request: &Request) -> Result<FetchedResource, ZoomError> {
        let headers = effective_request_headers(request, &self.user_header_names);
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
        let operation = registry.start(uri.into());
        self.drive(operation).await
    }

    /// Drive a previously-created pure discovery operation to completion.
    pub(crate) async fn drive(
        &self,
        mut operation: DiscoveryOperation,
    ) -> Result<ImageCatalog, NativeDiscoveryError> {
        loop {
            if operation.is_complete() {
                return operation.finish().map_err(Into::into);
            }
            let Some(next) = operation.next_priority_need()? else {
                return operation.finish().map_err(Into::into);
            };
            // Drive again after every supplied resource so a successful,
            // higher-priority candidate can finish before unrelated candidate
            // metadata is acquired.
            self.satisfy(&mut operation, next).await?;
        }
    }

    async fn satisfy(
        &self,
        operation: &mut DiscoveryOperation,
        need: ResourceNeed,
    ) -> Result<(), DiscoveryError> {
        match self.resolve(&need.request).await {
            Ok(resource) => operation.provide(
                ResourceResponse::new(need.id, resource.bytes).with_final_uri(resource.final_uri),
            ),
            Err(error) => operation.provide_failure(ResourceFailure {
                id: need.id,
                message: error.to_string(),
            }),
        }
    }
}
