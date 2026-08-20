//! Native application driver for the pure discovery core.
//!
//! The types in [`crate::core`] only describe resources.  This module is the
//! boundary which interprets those descriptions using reqwest or the local
//! filesystem.  Keeping the loop here also lets tests drive discovery with a
//! different resolver without pulling a runtime into the core.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use crate::ZoomError;
use crate::core::discovery::{
    DiscoveryError, DiscoveryOperation, RequestRequirements, ResourceFailure, ResourceNeed,
    ResourceResponse,
};
use crate::core::model::ImageCatalog;
use crate::core::registry::{Registry, RegistryError};
use crate::network::{FetchedResource, fetch_resource};

#[path = "google_arts_and_culture/decryption.rs"]
mod google_arts_decryption;

pub(crate) fn decrypt_google_arts_tile(
    data: Vec<u8>,
) -> Result<Vec<u8>, Box<dyn Error + Send + 'static>> {
    google_arts_decryption::decrypt(data)
        .map_err(|error| Box::new(error) as Box<dyn Error + Send + 'static>)
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ResourceCacheKey {
    uri: String,
    headers: Vec<(String, String)>,
}

impl ResourceCacheKey {
    fn new(uri: &str, requirements: &RequestRequirements) -> Self {
        Self {
            uri: uri.to_string(),
            headers: requirements
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        }
    }
}

/// Errors which can occur while the native application drives discovery.
#[derive(Debug)]
pub(crate) enum NativeDiscoveryError {
    Registry(RegistryError),
    Discovery(DiscoveryError),
}

impl fmt::Display for NativeDiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registry(error) => write!(f, "invalid discovery registry: {error}"),
            Self::Discovery(error) => write!(f, "discovery failed: {error}"),
        }
    }
}

impl Error for NativeDiscoveryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Registry(error) => Some(error),
            Self::Discovery(error) => Some(error),
        }
    }
}

impl From<RegistryError> for NativeDiscoveryError {
    fn from(error: RegistryError) -> Self {
        Self::Registry(error)
    }
}

impl From<DiscoveryError> for NativeDiscoveryError {
    fn from(error: DiscoveryError) -> Self {
        Self::Discovery(error)
    }
}

/// Operation-scoped native acquisition state.
///
/// A resolver may drive a root discovery operation and later a selected
/// deferred image.  Successful resources are reused across those operations,
/// matching the CLI's existing metadata-cache behavior without introducing
/// shared state into the core.
pub(crate) struct NativeDiscoveryDriver {
    http: reqwest::Client,
    cache: HashMap<ResourceCacheKey, FetchedResource>,
}

impl NativeDiscoveryDriver {
    pub(crate) fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            cache: HashMap::new(),
        }
    }

    /// Resolve a URI for the compatibility driver.
    pub(crate) async fn resolve_uri(
        &mut self,
        uri: &str,
        requirements: &RequestRequirements,
    ) -> Result<FetchedResource, ZoomError> {
        let key = ResourceCacheKey::new(uri, requirements);
        if let Some(resource) = self.cache.get(&key) {
            log::debug!("Using cached metadata for {uri}");
            return Ok(resource.clone());
        }

        let resource = fetch_resource(
            uri,
            &self.http,
            requirements
                .headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )
        .await?;
        self.cache.insert(key, resource.clone());
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
        match self.resolve_uri(&need.uri, &need.requirements).await {
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
