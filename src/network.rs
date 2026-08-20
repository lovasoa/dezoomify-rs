use std::collections::HashMap;
use std::future::Future;
use std::iter::once;
use std::path::PathBuf;
use std::sync::Arc;

use log::{debug, trace, warn};
use reqwest::{Client, header};
use sanitize_filename_reader_friendly::sanitize;
use tokio::fs;
use tokio::time::Duration;

use crate::arguments::Arguments;
use crate::binary_display::display_bytes;
use crate::dezoomer::{PostProcessFn, TileReference};
use crate::errors::BufferToImageError;
use crate::errors::{TileDownloadError, ZoomError};

/// Fetch data, either from an URL or a path to a local file.
/// If uri doesnt start with "http(s)://", it is considered to be a path
/// to a local file
// TODO: return Bytes
pub async fn fetch_uri(uri: &str, http: &Client) -> Result<Vec<u8>, ZoomError> {
    Ok(fetch_resource(uri, http, std::iter::empty()).await?.bytes)
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadedTile {
    pub position: crate::Vec2d,
    pub bytes: Arc<Vec<u8>>,
}

pub struct TileDownloader {
    pub http_client: reqwest::Client,
    pub post_process_fn: PostProcessFn,
    pub retries: usize,
    pub retry_delay: Duration,
    pub tile_storage_folder: Option<PathBuf>,
}

impl TileDownloader {
    pub async fn download_tile_and_then<T, F, Fut>(
        &self,
        tile_reference: TileReference,
        mut process: F,
    ) -> Result<T, TileDownloadError>
    where
        F: FnMut(DownloadedTile) -> Fut,
        Fut: Future<Output = Result<T, ZoomError>>,
    {
        let n = 100;
        let idx: f64 = ((tile_reference.position.x + tile_reference.position.y) % n).into();
        let tile_reference = Arc::new(tile_reference);
        let mut wait_time = self.retry_delay
            + Duration::from_secs_f64(idx * self.retry_delay.as_secs_f64() / f64::from(n));
        let mut failures: usize = 0;
        loop {
            let result = match self.load_tile_bytes(Arc::clone(&tile_reference)).await {
                Ok(tile) => process(tile).await,
                Err(cause) => Err(cause),
            };
            match result {
                Ok(processed) => return Ok(processed),
                Err(cause) => {
                    if failures >= self.retries {
                        return Err(TileDownloadError {
                            tile_reference: Arc::try_unwrap(tile_reference)
                                .expect("tile reference shouldn't leak"),
                            cause,
                        });
                    }
                    failures += 1;
                    warn!("{cause}. Retrying tile download in {wait_time:?}.");
                    tokio::time::sleep(wait_time).await;
                    wait_time *= 2;
                }
            }
        }
    }

    async fn load_tile_bytes(
        &self,
        tile_reference: Arc<TileReference>,
    ) -> Result<DownloadedTile, ZoomError> {
        let bytes = if let Some(bytes) = self.read_from_tile_cache(&tile_reference.url).await {
            bytes
        } else {
            let bytes = self
                .download_image_bytes(Arc::clone(&tile_reference))
                .await?;
            self.write_to_tile_cache(&tile_reference.url, &bytes).await;
            bytes
        };

        Ok(DownloadedTile {
            position: tile_reference.position,
            bytes: Arc::new(bytes),
        })
    }

    async fn download_image_bytes(
        &self,
        tile_reference: Arc<TileReference>,
    ) -> Result<Vec<u8>, ZoomError> {
        let mut bytes = fetch_uri(&tile_reference.url, &self.http_client).await?;
        if let PostProcessFn::Fn(post_process) = self.post_process_fn {
            bytes = tokio::task::spawn_blocking(move || -> Result<_, BufferToImageError> {
                post_process(&tile_reference, bytes)
                    .map_err(|e| BufferToImageError::PostProcessing { e })
            })
            .await??;
        }
        Ok(bytes)
    }

    async fn write_to_tile_cache(&self, uri: &str, contents: &[u8]) {
        if let Some(root) = &self.tile_storage_folder {
            match tokio::fs::write(root.join(sanitize(uri)), contents).await {
                Ok(()) => debug!("Wrote {} to tile cache ({} bytes)", uri, contents.len()),
                Err(e) => warn!(
                    "Unable to write {uri} to the tile cache {}: {e}",
                    root.display()
                ),
            }
        }
    }

    async fn read_from_tile_cache(&self, uri: &str) -> Option<Vec<u8>> {
        if let Some(root) = &self.tile_storage_folder {
            match tokio::fs::read(root.join(sanitize(uri))).await {
                Ok(d) => {
                    debug!("{uri} read from tile cache");
                    return Some(d);
                }
                Err(e) => debug!(
                    "Unable to open {uri} from tile cache {}: {e}",
                    root.display()
                ),
            }
        }
        None
    }
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
