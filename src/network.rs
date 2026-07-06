use std::collections::HashMap;
use std::iter::once;
use std::path::PathBuf;
use std::sync::Arc;

use log::{debug, trace, warn};
use reqwest::{Client, header};
use sanitize_filename_reader_friendly::sanitize;
use tokio::fs;
use tokio::time::Duration;
use url::Url;

use crate::arguments::Arguments;
use crate::binary_display::display_bytes;
use crate::dezoomer::{PostProcessFn, TileReference};
use crate::errors::BufferToImageError;
use crate::errors::{TileDownloadError, ZoomError};
use crate::tile::{EncodedTile, Tile, load_encoded_tile, load_image_with_metadata};

/// Fetch data, either from an URL or a path to a local file.
/// If uri doesnt start with "http(s)://", it is considered to be a path
/// to a local file
// TODO: return Bytes
pub async fn fetch_uri(uri: &str, http: &Client) -> Result<Vec<u8>, ZoomError> {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        let req = http.get(uri).build()?;
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
        let mut contents = Vec::new();
        let bytes = response.bytes().await?;
        contents.extend(bytes);
        trace!(
            "Successfully finished loading url: '{}' - received {} bytes: {}",
            uri,
            contents.len(),
            display_bytes(&contents[..contents.len().min(256)])
        );
        Ok(contents)
    } else {
        debug!("Loading file: '{uri}'");
        let result = fs::read(uri).await?;
        debug!(
            "Loaded file: '{}' - {} bytes: {}",
            uri,
            result.len(),
            display_bytes(&result[..result.len().min(256)])
        );
        Ok(result)
    }
}

pub struct TileDownloader {
    pub http_client: reqwest::Client,
    pub post_process_fn: PostProcessFn,
    pub retries: usize,
    pub retry_delay: Duration,
    pub tile_storage_folder: Option<PathBuf>,
}

impl TileDownloader {
    pub async fn download_tile(
        &self,
        tile_reference: TileReference,
    ) -> Result<Tile, TileDownloadError> {
        // The initial delay after which a failed request is retried depends on the position of the tile
        // in order to avoid sending repeated "bursts" of requests to a server that is struggling
        let n = 100;
        let idx: f64 = ((tile_reference.position.x + tile_reference.position.y) % n).into();
        let tile_reference = Arc::new(tile_reference);
        let mut wait_time = self.retry_delay
            + Duration::from_secs_f64(idx * self.retry_delay.as_secs_f64() / n as f64);
        let mut failures: usize = 0;
        loop {
            match self.load_image(Arc::clone(&tile_reference)).await {
                Ok(tile) => {
                    return Ok(tile);
                }
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

    pub async fn download_encoded_tile(
        &self,
        tile_reference: TileReference,
    ) -> Result<EncodedTile, TileDownloadError> {
        let n = 100;
        let idx: f64 = ((tile_reference.position.x + tile_reference.position.y) % n).into();
        let tile_reference = Arc::new(tile_reference);
        let mut wait_time = self.retry_delay
            + Duration::from_secs_f64(idx * self.retry_delay.as_secs_f64() / n as f64);
        let mut failures: usize = 0;
        loop {
            match self.load_encoded_image(Arc::clone(&tile_reference)).await {
                Ok(tile) => return Ok(tile),
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

    async fn load_image(&self, tile_reference: Arc<TileReference>) -> Result<Tile, ZoomError> {
        let bytes = if let Some(bytes) = self.read_from_tile_cache(&tile_reference.url).await {
            bytes
        } else {
            let bytes = self
                .download_image_bytes(Arc::clone(&tile_reference))
                .await?;
            self.write_to_tile_cache(&tile_reference.url, &bytes).await;
            bytes
        };

        let position = tile_reference.position;
        let image_with_metadata =
            tokio::task::spawn_blocking(move || load_image_with_metadata(&bytes)).await??;

        Ok(Tile::builder()
            .with_image(image_with_metadata.image)
            .at_position(position)
            .with_optional_icc_profile(image_with_metadata.icc_profile)
            .with_optional_exif_metadata(image_with_metadata.exif_metadata)
            .build())
    }

    async fn load_encoded_image(
        &self,
        tile_reference: Arc<TileReference>,
    ) -> Result<EncodedTile, ZoomError> {
        let bytes = if let Some(bytes) = self.read_from_tile_cache(&tile_reference.url).await {
            bytes
        } else {
            let bytes = self
                .download_image_bytes(Arc::clone(&tile_reference))
                .await?;
            self.write_to_tile_cache(&tile_reference.url, &bytes).await;
            bytes
        };

        let position = tile_reference.position;
        let bytes = Arc::new(bytes);
        let mut encoded = tokio::task::spawn_blocking(move || load_encoded_tile(bytes)).await??;
        encoded.position = position;
        Ok(encoded)
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
                Ok(_) => debug!("Wrote {} to tile cache ({} bytes)", uri, contents.len()),
                Err(e) => warn!("Unable to write {uri} to the tile cache {root:?}: {e}"),
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
                Err(e) => debug!("Unable to open {uri} from tile cache {root:?}: {e}"),
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
        .http1_title_case_headers()
        .default_headers(header_map)
        .referer(false)
        .pool_max_idle_per_host(args.max_idle_per_host)
        .danger_accept_invalid_certs(args.accept_invalid_certs)
        .timeout(args.timeout)
        .build()?;
    Ok(client)
}

pub fn default_headers() -> HashMap<String, String> {
    serde_yaml::from_str(include_str!("default_headers.yaml")).unwrap()
}

pub fn resolve_relative(base: &str, path: &str) -> String {
    if Url::parse(path).is_ok() {
        return path.to_string();
    } else if let Ok(url) = Url::parse(base)
        && let Ok(r) = url.join(path)
    {
        return r.to_string();
    }
    // Local-path fallback: drop the last component of `base` and append `path`.
    // Recognize both `/` and `\` so that Windows local paths resolve correctly
    // (a bare `C:\foo\bar\tour.js` has no `/`, so the old `/`-only split treated
    // the entire string as the directory and appended instead of replacing).
    //
    // Absolute paths (starting with `/` or a Windows drive prefix) replace the
    // base entirely, matching `PathBuf::push` semantics.
    if path.starts_with('/')
        || path.starts_with('\\')
        || (path.len() >= 2
            && path.as_bytes()[1] == b':'
            && path.as_bytes()[0].is_ascii_alphabetic())
    {
        return path.to_string();
    }
    let dir = base.rfind(['/', '\\']).map_or("", |idx| &base[..idx]);
    let dir = dir.trim_end_matches(['/', '\\']);
    if dir.is_empty() {
        path.to_string()
    } else {
        format!("{}/{}", dir, path)
    }
}

#[test]
fn test_resolve_relative() {
    assert_eq!(resolve_relative("/a/b", "c/d"), "/a/c/d");
    // Windows local path: the last component must be replaced, not appended.
    assert_eq!(resolve_relative("C:\\\\foo\\\\bar", "c/d"), "C:\\\\foo/c/d");
    assert_eq!(
        resolve_relative("C:\\\\foo\\\\bar\\\\tour.js", "tour.xml"),
        "C:\\\\foo\\\\bar/tour.xml"
    );
    assert_eq!(
        resolve_relative("/a/b", "http://example.com/x"),
        "http://example.com/x"
    );
    assert_eq!(
        resolve_relative("http://a.b", "http://example.com/x"),
        "http://example.com/x"
    );
    assert_eq!(resolve_relative("http://a.b", "c/d"), "http://a.b/c/d");
    assert_eq!(resolve_relative("http://a.b/x", "c/d"), "http://a.b/c/d");
    assert_eq!(resolve_relative("http://a.b/x/", "c/d"), "http://a.b/x/c/d");
    // Absolute local paths replace the base entirely.
    assert_eq!(
        resolve_relative("/metadata/tour.xml", "/tiles/0_0.jpg"),
        "/tiles/0_0.jpg"
    );
    // Absolute Windows paths replace the base entirely.
    assert_eq!(
        resolve_relative("C:\\metadata\\tour.xml", "C:\\tiles\\0_0.jpg"),
        "C:\\tiles\\0_0.jpg"
    );
}
