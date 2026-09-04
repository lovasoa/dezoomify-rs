//! Pure discovery for `XLimage` `*.img?cmd=info` documents.

use std::sync::Arc;

use serde::Deserialize;
use url::Url;

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryError, DiscoveryMatch, DiscoveryRoute, Grid, ImageCatalog,
    ImageDescriptor, LevelDescriptor, Request, StableId, resolve_relative,
};

const INFO_QUERY: &str = "cmd=info";

const ROUTES: &[DiscoveryRoute] = &[
    DiscoveryMatch::UrlPredicate(is_kbr_viewer).map_url(kbr_info_url),
    DiscoveryMatch::Any.extract(catalog),
];

pub const SPEC: DezoomerSpec = DezoomerSpec::new("xlimage", ROUTES)
    .recognizing(is_xlimage_url, "not an XLimage URL")
    .preferring(is_info_url);

fn is_xlimage_url(uri: &str) -> bool {
    let path = uri.split_once(['?', '#']).map_or(uri, |(path, _)| path);
    path.to_ascii_lowercase().contains(".img")
        && (path.to_ascii_lowercase().ends_with(".imgf")
            || path.to_ascii_lowercase().ends_with(".imgi")
            || path.to_ascii_lowercase().ends_with(".imgg"))
        || is_kbr_viewer(uri)
}

fn is_info_url(uri: &str) -> bool {
    uri.to_ascii_lowercase().contains(INFO_QUERY)
}

fn is_kbr_viewer(uri: &str) -> bool {
    kbr_viewer_id(uri).is_some()
}

fn kbr_viewer_id(uri: &str) -> Option<String> {
    let parsed = Url::parse(uri).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if host != "kbr.be" && !host.ends_with(".kbr.be") {
        return None;
    }
    let path = parsed.path();
    let path = path.strip_prefix("/multi/")?;
    let (id, _) = path.split_once("Viewer")?;
    (!id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    .then_some(id.to_owned())
}

fn kbr_info_url(input: &str) -> Result<Request, DiscoveryError> {
    let id = kbr_viewer_id(input)
        .ok_or_else(|| DiscoveryError::Session("invalid KBR XLimage viewer URL".into()))?;
    let mut viewer = Url::parse(input)
        .map_err(|_| DiscoveryError::Session("invalid KBR XLimage viewer URL".into()))?;
    let page = kbr_page_index(input)
        .checked_add(1)
        .ok_or_else(|| DiscoveryError::Session("KBR page number is too large".into()))?;
    viewer.set_path(&format!("/multi/{id}Viewer/xml.php"));
    viewer.set_query(None);
    viewer.set_fragment(None);
    Ok(Request::new(format!(
        "{viewer}?/multi/{id}/{page:03}.imgi?cmd=info"
    )))
}

fn kbr_page_index(uri: &str) -> usize {
    let Some(url) = Url::parse(uri).ok() else {
        return 0;
    };
    let from_pairs = |pairs: &str, wanted: &str| -> Option<usize> {
        pairs.split('&').find_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            name.eq_ignore_ascii_case(wanted)
                .then(|| value.parse().ok())?
        })
    };
    let from_query = |wanted: &str| {
        url.query_pairs().find_map(|(name, value)| {
            name.eq_ignore_ascii_case(wanted)
                .then(|| value.parse().ok())?
        })
    };
    from_query("dezoomify-page")
        .or_else(|| {
            url.fragment()
                .and_then(|fragment| from_pairs(fragment, "dezoomify-page"))
        })
        .or_else(|| from_query("page"))
        .or_else(|| {
            url.fragment()
                .and_then(|fragment| from_pairs(fragment, "page"))
        })
        .unwrap_or(0)
}

fn image_origin(url: &str) -> String {
    let (path, query) = url.split_once('?').map_or((url, None), |(path, query)| {
        (
            path,
            Some(query.split_once('#').map_or(query, |(query, _)| query)),
        )
    });
    if let Some(nested_path) = query
        .and_then(|query| query.split_once('?').map(|(path, _)| path))
        .filter(|path| path.to_ascii_lowercase().contains(".img"))
    {
        return format!("{path}?{nested_path}");
    }
    if path.to_ascii_lowercase().contains(".img") {
        return path.to_owned();
    }
    query
        .and_then(|query| query.split_once('?').map(|(path, _)| path))
        .filter(|path| path.to_ascii_lowercase().contains(".img"))
        .map_or_else(|| path.to_owned(), |path| resolve_relative(url, path))
}

fn catalog(url: &str, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let metadata: Metadata = serde_xml_rs::from_reader(bytes).map_err(|error| {
        DiscoveryError::Session(format!("unable to parse XLimage metadata: {error}"))
    })?;
    if metadata.width == 0
        || metadata.height == 0
        || metadata.tileside == 0
        || metadata.maxzoom == 0
    {
        return Err(DiscoveryError::Session(
            "XLimage metadata must declare positive width, height, tileside, and maxzoom".into(),
        ));
    }
    let origin: Arc<str> = image_origin(url).into();
    let levels = build_levels(&metadata, &origin)?;

    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("xlimage:image"),
        title: Some("XLimage".into()),
        format: StableId::new("xlimage"),
        levels,
        ..Default::default()
    })]))
}

fn build_levels(
    metadata: &Metadata,
    origin: &Arc<str>,
) -> Result<Vec<LevelDescriptor>, DiscoveryError> {
    let mut levels = Vec::new();
    let mut zoom = 1;
    loop {
        let width = metadata.width.div_ceil(zoom);
        let height = metadata.height.div_ceil(zoom);
        let origin = Arc::clone(origin);
        let source = Grid::with_requests(
            StableId::new(format!("xlimage:{zoom}")),
            Vec2d {
                x: width,
                y: height,
            },
            Vec2d::square(metadata.tileside),
            Vec2d::default(),
            move |tile| {
                let coord: Vec2d = tile.coord.into();
                Request::new(format!(
                    "{origin}?cmd=tile&x={}&y={}&z={zoom}",
                    coord.x, coord.y
                ))
            },
        )
        .map_err(|error| DiscoveryError::Session(format!("invalid XLimage grid: {error}")))?;
        levels.push(LevelDescriptor::new(source).with_scale_factor(Some(zoom)));

        if zoom >= metadata.maxzoom {
            break;
        }
        zoom = zoom
            .checked_mul(2)
            .map_or(metadata.maxzoom, |next| next.min(metadata.maxzoom));
    }
    Ok(levels)
}

#[derive(Debug, Deserialize)]
struct Metadata {
    width: u32,
    height: u32,
    tileside: u32,
    #[serde(default = "default_maxzoom")]
    maxzoom: u32,
}

fn default_maxzoom() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kbr_viewer_uses_the_requested_page_and_broker_for_tiles() {
        let request = kbr_info_url("https://kbr.be/multi/abc_defViewer/index.html#page=3").unwrap();
        assert_eq!(
            request.uri,
            "https://kbr.be/multi/abc_defViewer/xml.php?/multi/abc_def/004.imgi?cmd=info"
        );
        assert_eq!(
            image_origin(&request.uri),
            "https://kbr.be/multi/abc_defViewer/xml.php?/multi/abc_def/004.imgi"
        );
    }

    #[test]
    fn cli_page_hint_overrides_page_in_the_viewer_url() {
        for input in [
            "https://kbr.be/multi/abc_defViewer/index.html?page=1#dezoomify-page=3",
            "https://kbr.be/multi/abc_defViewer/index.html#page=1&dezoomify-page=3",
        ] {
            assert_eq!(
                kbr_info_url(input).unwrap().uri,
                "https://kbr.be/multi/abc_defViewer/xml.php?/multi/abc_def/004.imgi?cmd=info"
            );
        }
    }
}
