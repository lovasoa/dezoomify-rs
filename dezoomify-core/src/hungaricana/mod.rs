//! Pure discovery for Hungaricana ECW image services.

use std::sync::{Arc, LazyLock};

use aes::cipher::{BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
use regex::Regex;
use serde::Deserialize;
use url::Url;

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryContext, DiscoveryError, DiscoveryMatch,
    DiscoveryResource, DiscoveryRoute, DiscoveryStep, Grid, ImageCatalog, ImageDescriptor,
    LevelDescriptor, Request, StableId, resolve_relative,
};

static LAYER_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)layer_?url[\"']?\s*:\s*[\"']([^\"']+)"#)
        .expect("constant Hungaricana layer URL pattern")
});
static PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:^|[^A-Za-z0-9_])path[\"']?\s*:\s*[\"']([^\"']*)"#)
        .expect("constant Hungaricana path pattern")
});
static IMAGE_PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)imagepath[\"']?\s*=\s*[\"']([^\"']+)"#)
        .expect("constant Hungaricana image path pattern")
});
static FILES_ARRAY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)(?:files|images)[\"']?\s*:\s*(\[.*?\])"#)
        .expect("constant Hungaricana file array pattern")
});
static FILES_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)files_url[\"']?\s*:\s*[\"']([^\"']+)"#)
        .expect("constant Hungaricana file URL pattern")
});

const ROUTES: &[DiscoveryRoute] = &[
    DiscoveryMatch::UrlPredicate(is_ecw_url).extract(catalog),
    DiscoveryMatch::UrlPredicate(is_files_url).then(follow_files),
    DiscoveryMatch::ContentPredicate(contains_layer).then(follow_layer),
    DiscoveryMatch::ContentPredicate(contains_files).then(follow_files),
    DiscoveryMatch::Any.extract(catalog),
];

pub const SPEC: DezoomerSpec = DezoomerSpec::new("hungaricana", ROUTES)
    .recognizing(is_hungaricana_url, "not a Hungaricana URL")
    .preferring(|uri| uri.to_ascii_lowercase().contains("hungaricana"));

fn is_hungaricana_url(uri: &str) -> bool {
    let lower = uri.to_ascii_lowercase();
    lower.contains("hungaricana") || is_ecw_url(uri)
}

fn is_ecw_url(uri: &str) -> bool {
    uri.split_once(['?', '#'])
        .map_or(uri, |(path, _)| path)
        .to_ascii_lowercase()
        .ends_with(".ecw")
}

fn is_files_url(uri: &str) -> bool {
    uri.split_once(['?', '#'])
        .map_or(uri, |(path, _)| path)
        .to_ascii_lowercase()
        .ends_with("files.json")
}

fn contains_layer(bytes: &[u8]) -> bool {
    LAYER_URL_RE.is_match(&String::from_utf8_lossy(bytes))
}

fn contains_files(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Vec<String>>(bytes).is_ok()
}

fn follow_layer(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let page = resource.text_lossy();
    if let Some(file) = IMAGE_PATH_RE
        .captures(&page)
        .and_then(|captures| captures.get(1))
    {
        return Ok(DiscoveryStep::Follow(Request::new(layer_file_url(
            &page,
            file.as_str(),
        )?)));
    }
    if let Some(files) = FILES_ARRAY_RE
        .captures(&page)
        .and_then(|captures| captures.get(1))
    {
        return follow_file_array(&page, resource.final_uri(), files.as_str());
    }
    if let Some(files_url) = FILES_URL_RE
        .captures(&page)
        .and_then(|captures| captures.get(1))
    {
        return Ok(DiscoveryStep::Follow(Request::new(resolve_relative(
            resource.final_uri(),
            files_url.as_str(),
        ))));
    }
    Err(DiscoveryError::Session(
        "unable to find the Hungaricana layer file name".into(),
    ))
}

fn follow_files(
    context: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let files: Vec<String> = serde_json::from_slice(resource.bytes()).map_err(|error| {
        DiscoveryError::Session(format!("invalid Hungaricana file list: {error}"))
    })?;
    let page = context
        .resources()
        .rev()
        .find(|candidate| contains_layer(candidate.bytes()))
        .ok_or_else(|| DiscoveryError::Session("Hungaricana layer page is missing".into()))?;
    let page_text = page.text_lossy();
    let index = image_index(page.final_uri());
    let file = files
        .get(index)
        .ok_or_else(|| DiscoveryError::Session("Hungaricana file index is out of range".into()))?;
    Ok(DiscoveryStep::Follow(Request::new(layer_file_url(
        &page_text, file,
    )?)))
}

fn follow_file_array(
    page: &str,
    page_uri: &str,
    encoded: &str,
) -> Result<DiscoveryStep, DiscoveryError> {
    let files: Vec<String> = serde_json::from_str(encoded).map_err(|error| {
        DiscoveryError::Session(format!("invalid Hungaricana file list: {error}"))
    })?;
    let file = files
        .get(image_index(page_uri))
        .ok_or_else(|| DiscoveryError::Session("Hungaricana file index is out of range".into()))?;
    Ok(DiscoveryStep::Follow(Request::new(layer_file_url(
        page, file,
    )?)))
}

fn layer_file_url(page: &str, file: &str) -> Result<String, DiscoveryError> {
    let layer = LAYER_URL_RE
        .captures(page)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().replace("&amp;", "&"))
        .ok_or_else(|| DiscoveryError::Session("Hungaricana page has no layer URL".into()))?;
    let path = PATH_RE
        .captures(page)
        .and_then(|captures| captures.get(1))
        .map_or("", |value| value.as_str());
    Ok(format!("{layer}{path}{file}"))
}

fn image_index(uri: &str) -> usize {
    Url::parse(uri)
        .ok()
        .and_then(|url| {
            url.query_pairs().find_map(|(name, value)| {
                matches!(name.as_ref(), "img" | "pg")
                    .then(|| value.parse::<usize>().ok())
                    .flatten()
            })
        })
        .unwrap_or(0)
}

fn catalog(url: &str, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let metadata: Metadata = serde_json::from_slice(bytes).map_err(|error| {
        DiscoveryError::Session(format!(
            "unable to parse Hungaricana image metadata: {error}"
        ))
    })?;
    if metadata.width == 0 || metadata.height == 0 {
        return Err(DiscoveryError::Session(
            "Hungaricana image dimensions must be positive".into(),
        ));
    }
    let (origin, path): (String, String) = if let Some((base, path)) = url.split_once("imagesize/")
    {
        (format!("{base}image/{path}/"), path.to_owned())
    } else if let Some((base, path)) = url.split_once("/image/") {
        (format!("{base}/image/{path}/"), path.to_owned())
    } else {
        return Err(DiscoveryError::Session(
            "Hungaricana metadata URL has no image path".into(),
        ));
    };
    let origin: Arc<str> = origin.into();
    let tile_path: Arc<str> = path.clone().into();
    let zoom = max_zoom(metadata.width.max(metadata.height), 512);
    let source = Grid::with_requests(
        StableId::new("hungaricana:level"),
        Vec2d {
            x: metadata.width,
            y: metadata.height,
        },
        Vec2d::square(512),
        Vec2d::default(),
        move |tile| {
            let hash = tile_hash(tile.coord.column, tile.coord.row, zoom, &tile_path);
            Request::new(format!("{origin}{hash}"))
        },
    )
    .map_err(|error| DiscoveryError::Session(format!("invalid Hungaricana grid: {error}")))?;
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("hungaricana:image"),
        title: Some(path.clone()),
        format: StableId::new("hungaricana"),
        levels: vec![LevelDescriptor::new(source)],
        ..Default::default()
    })]))
}

#[derive(Debug, Deserialize)]
struct Metadata {
    width: u32,
    height: u32,
}

fn max_zoom(size: u32, tile_size: u32) -> u32 {
    let mut zoom = 0;
    let mut covered = u64::from(tile_size);
    while covered < u64::from(size) {
        covered *= 2;
        zoom += 1;
    }
    zoom
}

fn tile_hash(x: u32, y: u32, z: u32, base_path: &str) -> String {
    const KEY: &[u8; 16] = b"dGhpcyBpcyBubyBr";
    const IV: [u8; 16] = [0; 16];
    let sum = path_sum(base_path);
    let plaintext = format!("{}|{z}|{x}|{y}", sum % 100);
    let mut buffer = [0_u8; 64];
    buffer[..16].fill(b'*');
    buffer[..plaintext.len()].copy_from_slice(plaintext.as_bytes());
    let encrypted = cbc::Encryptor::<aes::Aes128>::new(KEY.into(), (&IV).into())
        .encrypt_padded::<Pkcs7>(&mut buffer, 16)
        .expect("Hungaricana hash input fits in the local AES buffer");
    encrypted[..16]
        .iter()
        .fold(String::with_capacity(32), |mut hash, byte| {
            use std::fmt::Write as _;
            write!(hash, "{byte:02x}").expect("writing a hash to String cannot fail");
            hash
        })
}

fn path_sum(path: &str) -> u64 {
    path.encode_utf16().map(u64::from).sum()
}

#[cfg(test)]
mod tests {
    use super::path_sum;

    #[test]
    fn hash_path_sum_matches_javascript_utf16_units() {
        assert_eq!(path_sum("a😀"), 112_286);
    }
}
