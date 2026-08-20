//! Native application driver for the pure discovery core.
//!
//! The types in [`dezoomify_core::core`] only describe resources.  This module
//! is the boundary which interprets those descriptions using reqwest or the
//! local filesystem.  Keeping the loop here also lets tests drive discovery
//! with a different resolver without pulling a runtime into the core.

use std::collections::HashMap;
use std::error::Error;

use crate::ZoomError;
use crate::network::{FetchedResource, fetch_resource, request_headers};
use custom_error::custom_error;
use dezoomify_core::core::discovery::{
    DiscoveryError, DiscoveryOperation, ResourceFailure, ResourceNeed, ResourceResponse,
};
use dezoomify_core::core::model::{ImageCatalog, Request};
use dezoomify_core::core::registry::{Registry, RegistryError};

#[path = "google_arts_decryption.rs"]
mod google_arts_decryption;

pub(crate) fn decrypt_google_arts_tile(
    data: Vec<u8>,
) -> Result<Vec<u8>, Box<dyn Error + Send + 'static>> {
    google_arts_decryption::decrypt(data)
        .map_err(|error| Box::new(error) as Box<dyn Error + Send + 'static>)
}

// Errors which can occur while the native application drives discovery.
custom_error! {pub NativeDiscoveryError
    Registry{source: RegistryError} = "invalid discovery registry: {source}",
    Discovery{source: DiscoveryError} = "discovery failed: {source}",
}

/// Operation-scoped native acquisition state.
///
/// A resolver may drive a root discovery operation and later a selected
/// deferred image.  Successful resources are reused across those operations,
/// matching the CLI's existing metadata-cache behavior without introducing
/// shared state into the core.
pub(crate) struct NativeDiscoveryDriver {
    http: reqwest::Client,
    cache: HashMap<Request, FetchedResource>,
}

impl NativeDiscoveryDriver {
    pub(crate) fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            cache: HashMap::new(),
        }
    }

    /// Resolve one canonical resource request, reusing matching metadata.
    pub(crate) async fn resolve(
        &mut self,
        request: &Request,
    ) -> Result<FetchedResource, ZoomError> {
        if let Some(resource) = self.cache.get(request) {
            log::debug!("Using cached metadata for {}", request.uri);
            return Ok(resource.clone());
        }

        let headers = request_headers(request);
        let resource = fetch_resource(
            &request.uri,
            &self.http,
            headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )
        .await?;
        self.cache.insert(request.clone(), resource.clone());
        Ok(resource)
    }

    /// Start and drive discovery for an input using the supplied pure registry.
    pub(crate) async fn discover(
        &mut self,
        registry: &Registry,
        uri: impl Into<String>,
    ) -> Result<ImageCatalog, NativeDiscoveryError> {
        let operation = registry.start(uri.into())?;
        self.drive(operation).await
    }

    /// Drive a previously-created pure discovery operation to completion.
    pub(crate) async fn drive(
        &mut self,
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
        &mut self,
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
