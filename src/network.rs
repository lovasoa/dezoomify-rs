use std::collections::HashMap;
use std::fmt::Write;
use std::future::Future;
use std::iter::once;
use std::path::PathBuf;
use std::sync::Arc;

use log::{debug, trace, warn};
use reqwest::{Client, header};
use sanitize_filename_reader_friendly::sanitize;
use sha1::{Digest, Sha1};
use tokio::fs;
use tokio::time::Duration;

use crate::arguments::Arguments;
use crate::binary_display::display_bytes;
use crate::errors::BufferToImageError;
use crate::errors::{TileDownloadError, ZoomError};
use dezoomify_core::core::{ProcessingRecipe, Request, TileSpec};

/// Bytes and portable response metadata acquired by the native application.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FetchedResource {
    pub(crate) bytes: Vec<u8>,
    pub(crate) content_type: Option<String>,
}

/// Fetch a described resource with request-specific headers.
///
/// Header strings remain neutral in the core and are converted to reqwest
/// values only at this native boundary.  Local files intentionally ignore HTTP
/// requirements while preserving the same URI behavior as [`fetch_uri`].
pub(crate) async fn fetch_resource<'a>(
    uri: &str,
    http: &Client,
    headers: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<FetchedResource, ZoomError> {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        let mut request = http.get(uri);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let req = request.build()?;
        debug!(
            "Making http request to {uri} with headers '{:?}'",
            req.headers()
        );
        let response = http.execute(req).await?;
        debug!(
            "Got http response for {uri}: status={},  headers={:?}",
            response.status(),
            response.headers()
        );
        let response = response.error_for_status()?;
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let mut contents = Vec::new();
        let bytes = response.bytes().await?;
        contents.extend(bytes);
        trace!(
            "Successfully finished loading url: '{}' - received {} bytes: {}",
            uri,
            contents.len(),
            display_bytes(&contents[..contents.len().min(256)])
        );
        Ok(FetchedResource {
            bytes: contents,
            content_type,
        })
    } else {
        debug!("Loading file: '{uri}'");
        let result = fs::read(uri).await?;
        debug!(
            "Loaded file: '{}' - {} bytes: {}",
            uri,
            result.len(),
            display_bytes(&result[..result.len().min(256)])
        );
        Ok(FetchedResource {
            bytes: result,
            content_type: None,
        })
    }
}

pub(crate) fn request_headers(request: &Request) -> Vec<(String, String)> {
    let mut headers = request.headers.clone();
    if !request.accepted_content_types.is_empty()
        && !headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("accept"))
    {
        headers.insert(
            "Accept".to_owned(),
            request
                .accepted_content_types
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    headers.into_iter().collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadedTile {
    pub spec: TileSpec,
    pub bytes: Arc<Vec<u8>>,
}

pub struct TileDownloader {
    pub http_client: reqwest::Client,
    pub retries: usize,
    pub retry_delay: Duration,
    pub tile_storage_folder: Option<PathBuf>,
}

impl TileDownloader {
    pub async fn download_tile_and_then<T, F, Fut>(
        &self,
        tile_spec: TileSpec,
        mut process: F,
    ) -> Result<T, TileDownloadError>
    where
        F: FnMut(DownloadedTile) -> Fut,
        Fut: Future<Output = Result<T, ZoomError>>,
    {
        let n = 100;
        let idx: f64 = ((tile_spec.destination.x + tile_spec.destination.y) % n).into();
        let mut wait_time = self.retry_delay
            + Duration::from_secs_f64(idx * self.retry_delay.as_secs_f64() / f64::from(n));
        let mut failures: usize = 0;
        loop {
            let result = match self.load_tile_bytes(&tile_spec).await {
                Ok(tile) => process(tile).await,
                Err(cause) => Err(cause),
            };
            match result {
                Ok(processed) => return Ok(processed),
                Err(cause) => {
                    if failures >= self.retries {
                        return Err(TileDownloadError { tile_spec, cause });
                    }
                    failures += 1;
                    warn!("{cause}. Retrying tile download in {wait_time:?}.");
                    tokio::time::sleep(wait_time).await;
                    wait_time *= 2;
                }
            }
        }
    }

    async fn load_tile_bytes(&self, tile_spec: &TileSpec) -> Result<DownloadedTile, ZoomError> {
        let bytes = if let Some(bytes) = self.read_from_tile_cache(&tile_spec.request).await {
            bytes
        } else {
            let bytes = self.download_image_bytes(tile_spec).await?;
            self.write_to_tile_cache(&tile_spec.request, &bytes).await;
            bytes
        };

        Ok(DownloadedTile {
            spec: tile_spec.clone(),
            bytes: Arc::new(bytes),
        })
    }

    async fn download_image_bytes(&self, tile_spec: &TileSpec) -> Result<Vec<u8>, ZoomError> {
        let headers = request_headers(&tile_spec.request);
        let mut bytes = fetch_resource(
            &tile_spec.request.uri,
            &self.http_client,
            headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )
        .await?
        .bytes;
        match &tile_spec.processing {
            ProcessingRecipe::None => {}
            ProcessingRecipe::Named(name) if name.as_str() == "google-arts-decrypt" => {
                bytes = tokio::task::spawn_blocking(move || {
                    crate::native::decrypt_google_arts_tile(bytes)
                        .map_err(|e| BufferToImageError::PostProcessing { e })
                })
                .await??;
            }
            ProcessingRecipe::Named(name) => {
                return Err(ZoomError::UnsupportedProcessingRecipe {
                    name: name.to_string(),
                });
            }
        }
        Ok(bytes)
    }

    async fn write_to_tile_cache(&self, request: &Request, contents: &[u8]) {
        if let Some(root) = &self.tile_storage_folder {
            match tokio::fs::write(tile_cache_path(root, request), contents).await {
                Ok(()) => debug!(
                    "Wrote {} to tile cache ({} bytes)",
                    request.uri,
                    contents.len()
                ),
                Err(e) => warn!(
                    "Unable to write {} to the tile cache {}: {e}",
                    request.uri,
                    root.display()
                ),
            }
        }
    }

    async fn read_from_tile_cache(&self, request: &Request) -> Option<Vec<u8>> {
        if let Some(root) = &self.tile_storage_folder {
            let paths = tile_cache_paths(root, request);
            for (index, path) in paths.iter().enumerate() {
                match tokio::fs::read(path).await {
                    Ok(d) => {
                        if index == 0 {
                            debug!("{} read from tile cache", request.uri);
                        } else {
                            debug!("{} read from legacy tile cache", request.uri);
                        }
                        return Some(d);
                    }
                    Err(e) => debug!(
                        "Unable to open {} from tile cache {}: {e}",
                        request.uri,
                        root.display()
                    ),
                }
            }
        }
        None
    }
}

fn tile_cache_path(root: &std::path::Path, request: &Request) -> PathBuf {
    let mut digest = Sha1::new();
    digest.update(request.uri.as_bytes());
    for (name, value) in &request.headers {
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    for content_type in &request.accepted_content_types {
        digest.update(content_type.as_bytes());
        digest.update([0]);
    }
    let digest = digest
        .finalize()
        .iter()
        .fold(String::with_capacity(40), |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a string cannot fail");
            output
        });
    root.join(format!("{}-{digest}", sanitize(&request.uri)))
}

fn legacy_tile_cache_path(root: &std::path::Path, request: &Request) -> PathBuf {
    root.join(sanitize(&request.uri))
}

fn tile_cache_paths(root: &std::path::Path, request: &Request) -> [PathBuf; 2] {
    [
        tile_cache_path(root, request),
        legacy_tile_cache_path(root, request),
    ]
}

pub fn client<'a, I: Iterator<Item = (&'a String, &'a String)>>(
    headers: I,
    args: &Arguments,
    uri: Option<&str>,
) -> Result<reqwest::Client, ZoomError> {
    let referer = uri.or(args.request_referer()).unwrap_or("");
    let header_map = default_headers()
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .chain(once(("Referer", referer)))
        .chain(headers.map(|(k, v)| (&**k, &**v)))
        .map(|(name, value)| Ok((name.parse()?, value.parse()?)))
        .collect::<Result<header::HeaderMap, ZoomError>>()?;
    debug!("Creating an http client with the following headers: {header_map:?}");
    let client = reqwest::Client::builder()
        .use_native_tls()
        .http1_title_case_headers()
        .default_headers(header_map)
        .referer(false)
        .pool_max_idle_per_host(args.max_idle_per_host)
        .danger_accept_invalid_certs(args.accept_invalid_certs)
        .connect_timeout(args.connect_timeout)
        .timeout(args.timeout)
        .build()?;
    Ok(client)
}

pub fn default_headers() -> HashMap<String, String> {
    serde_yaml::from_str(include_str!("default_headers.yaml")).unwrap()
}

#[cfg(test)]
mod tests {
    use super::{Request, TileDownloader, request_headers, tile_cache_path, tile_cache_paths};
    use std::path::Path;
    use tokio::time::Duration;

    #[test]
    fn tile_cache_identity_includes_all_request_requirements() {
        let first = Request::new("https://example.test/tile").with_header("X-Key", "one");
        let second = Request::new("https://example.test/tile").with_header("X-Key", "two");
        assert_ne!(
            tile_cache_path(Path::new("/tmp/cache"), &first),
            tile_cache_path(Path::new("/tmp/cache"), &second)
        );
        let mut png = Request::new("https://example.test/tile");
        png.accepted_content_types.insert("image/png".into());
        let mut jpeg = Request::new("https://example.test/tile");
        jpeg.accepted_content_types.insert("image/jpeg".into());
        assert_ne!(
            tile_cache_path(Path::new("/tmp/cache"), &png),
            tile_cache_path(Path::new("/tmp/cache"), &jpeg)
        );
    }

    #[test]
    fn tile_cache_paths_try_hashed_path_before_legacy_path() {
        let request = Request::new("https://example.test/tile");
        let paths = tile_cache_paths(Path::new("/tmp/cache"), &request);
        assert_eq!(paths[0], tile_cache_path(Path::new("/tmp/cache"), &request));
        assert_eq!(
            paths[1],
            Path::new("/tmp/cache").join("https_example.test_tile")
        );
        assert_ne!(paths[0], paths[1]);
    }

    #[tokio::test]
    async fn tile_cache_reads_legacy_entries_and_prefers_hashed_entries() {
        let directory = tempfile::tempdir().unwrap();
        let request = Request::new("https://example.test/tile");
        let paths = tile_cache_paths(directory.path(), &request);
        tokio::fs::write(&paths[1], b"legacy").await.unwrap();

        let downloader = TileDownloader {
            http_client: reqwest::Client::new(),
            retries: 0,
            retry_delay: Duration::ZERO,
            tile_storage_folder: Some(directory.path().to_path_buf()),
        };
        assert_eq!(
            downloader.read_from_tile_cache(&request).await,
            Some(b"legacy".to_vec())
        );

        tokio::fs::write(&paths[0], b"hashed").await.unwrap();
        assert_eq!(
            downloader.read_from_tile_cache(&request).await,
            Some(b"hashed".to_vec())
        );
    }

    #[test]
    fn accepted_content_types_supply_default_accept_header() {
        let mut request = Request::new("memory://tile");
        request
            .accepted_content_types
            .insert("image/png".to_owned());
        assert_eq!(
            request_headers(&request),
            vec![("Accept".to_owned(), "image/png".to_owned())]
        );
    }
}
