use std::collections::HashMap;
use std::iter::once;
use std::path::PathBuf;
use std::sync::Arc;

use image::DynamicImage;
use log::{debug, warn};
use reqwest::{Client, header};
use sanitize_filename_reader_friendly::sanitize;
use tokio::fs;
use tokio::time::Duration;
use url::Url;

use crate::{TileDownloadError, ZoomError};
use crate::arguments::Arguments;
use crate::dezoomer::{PostProcessFn, TileReference};
use crate::errors::BufferToImageError;
use crate::tile::Tile;

/// Fetch data, either from an URL or a path to a local file.
/// If uri doesnt start with "http(s)://", it is considered to be a path
/// to a local file
// TODO: return Bytes
pub async fn fetch_uri(uri: &str, http: &Client) -> Result<Vec<u8>, ZoomError> {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        debug!("Loading url: '{}'", uri);
        let response = http.get(uri).send()
            .await?.error_for_status()?;
        let mut contents = Vec::new();
        let bytes = response.bytes().await?;
        contents.extend(bytes);
        debug!("Loaded url: '{}'", uri);
        Ok(contents)
    } else {
        debug!("Loading file: '{}'", uri);
        let result = fs::read(uri).await?;
        debug!("Loaded file: '{}'", uri);
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
        let mut wait_time = self.retry_delay + Duration::from_secs_f64(idx * self.retry_delay.as_secs_f64() / n as f64);
        let mut failures: usize = 0;
        loop {
            match self.load_image(Arc::clone(&tile_reference)).await {
                Ok(image) => {
                    return Ok(Tile { image, position: tile_reference.position })
                },
                Err(cause) => {
                    if failures >= self.retries {
                        return Err(TileDownloadError {
                            tile_reference: Arc::try_unwrap(tile_reference)
                                .expect("tile reference shouldn't leak"),
                            cause,
                        })
                    }
                    failures += 1;
                    warn!("{}. Retrying tile download in {:?}.", cause, wait_time);
                    tokio::time::sleep(wait_time).await;
                    wait_time *= 2;
                }
            }
        }
    }

    async fn load_image(
        &self,
        tile_reference: Arc<TileReference>,
    ) -> Result<DynamicImage, ZoomError> {
        let bytes =
            if let Some(bytes) = self.read_from_tile_cache(&tile_reference.url).await {
                bytes
            } else {
                let bytes = self.download_image_bytes(Arc::clone(&tile_reference)).await?;
                self.write_to_tile_cache(&tile_reference.url, &bytes).await;
                bytes
            };
        Ok(tokio::task::spawn_blocking(move || {
            image::load_from_memory(&bytes)
        }).await??)
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
            }).await??;
        }
        Ok(bytes)
    }

    async fn write_to_tile_cache(&self, uri: &str, contents: &[u8]) {
        if let Some(root) = &self.tile_storage_folder {
            match tokio::fs::write(root.join(&sanitize(uri)), contents).await {
                Ok(_) => debug!("Wrote {} to tile cache ({} bytes)", uri, contents.len()),
                Err(e) => warn!("Unable to write {} to the tile cache {:?}: {}", uri, root, e)
            }
        }
    }

    async fn read_from_tile_cache(&self, uri: &str) -> Option<Vec<u8>> {
        if let Some(root) = &self.tile_storage_folder {
            match tokio::fs::read(root.join(&sanitize(uri))).await {
                Ok(d) => {
                    debug!("{} read from tile cache", uri);
                    return Some(d);
                },
                Err(e) => debug!("Unable to open {} from tile cache {:?}: {}", uri, root, e)
            }
        }
        None
    }
}

pub fn client<'a, I: Iterator<Item=(&'a String, &'a String)>>(
    headers: I,
    args: &Arguments,
    uri: Option<&str>,
) -> Result<reqwest::Client, ZoomError> {
    let referer = uri.or(args.input_uri.as_deref()).unwrap_or("").to_string();
    let header_map = default_headers()
        .iter()
        .chain(once((&"Referer".to_string(), &referer)))
        .chain(headers.map(|(k, v)| (k, v)))
        .map(|(name, value)| Ok((name.parse()?, value.parse()?)))
        .collect::<Result<header::HeaderMap, ZoomError>>()?;
    debug!("Creating an http client with the following headers: {:?}", header_map);
    let client = reqwest::Client::builder()
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
        return path.to_string()
    } else if let Ok(url) = Url::parse(base) {
        if let Ok(r) = url.join(path) {
            return r.to_string()
        }
    }
    let mut res = PathBuf::from(base.rsplitn(2, '/').last().unwrap_or_default());
    res.push(path);
    res.to_string_lossy().to_string()
}

pub fn remove_bom(contents: &[u8]) -> &[u8] {
    // Workaround for https://github.com/netvl/xml-rs/issues/155
    // which the original author seems unwilling to fix
    const BOM: &[u8] = &[0xEF, 0xBB, 0xBF]; // UTF8 byte order mark
    if contents.starts_with(BOM) {
        &contents[BOM.len()..]
    } else { contents }
}

#[test]
fn test_resolve_relative() {
    use std::path::MAIN_SEPARATOR;
    assert_eq!(resolve_relative("/a/b", "c/d"), format!("/a{}c/d", MAIN_SEPARATOR));
    assert_eq!(resolve_relative("C:\\X", "c/d"), format!("C:\\X{}c/d", MAIN_SEPARATOR));
    assert_eq!(resolve_relative("/a/b", "http://example.com/x"), "http://example.com/x");
    assert_eq!(resolve_relative("http://a.b", "http://example.com/x"), "http://example.com/x");
    assert_eq!(resolve_relative("http://a.b", "c/d"), "http://a.b/c/d");
    assert_eq!(resolve_relative("http://a.b/x", "c/d"), "http://a.b/c/d");
    assert_eq!(resolve_relative("http://a.b/x/", "c/d"), "http://a.b/x/c/d");
}