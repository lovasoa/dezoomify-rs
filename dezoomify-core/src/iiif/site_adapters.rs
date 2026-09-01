use std::sync::LazyLock;

use regex::bytes::Regex as BytesRegex;
use url::Url;

use crate::core::{
    DiscoveryContext, DiscoveryError, DiscoveryResource, DiscoveryStep, Request, resolve_relative,
};

static MICRIO_CUSTOM_ELEMENT: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(r#"(?is)<micr-io\b[^>]*\bid\s*=\s*["'](?P<id>[A-Za-z0-9]{5})["']"#)
        .expect("constant Micrio custom element pattern")
});
static NATIONAL_GALLERY_IMAGE: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(
        r#"(?is)(?P<image>(?:https?://|/|\./|\.\./)[^"'\s<>]+\?IIIF=[^"'\s<>]+?/full/[^"'\s<>]+)"#,
    )
    .expect("constant National Gallery IIIF image pattern")
});
static LONDON_MUSEUM_SERVICE: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(
        r#"(?is)(?:src|data-src)\s*=\s*["'](?P<service>https?://collections\.londonmuseum\.net/iiif/3/[^"'\s<>]+?\.ptif)"#,
    )
    .expect("constant London Museum IIIF service pattern")
});
static PHILADELPHIA_MICRIO: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(r#"(?is)(?:philamuseum|philadelphia museum).*?\\?"shortId\\?"\s*:\s*\\?"(?P<id>[A-Za-z0-9_-]{3,32})\\?""#)
        .expect("constant Philadelphia Museum Micrio pattern")
});

pub(super) fn is_onb_entry(uri: &str) -> bool {
    let Ok(url) = Url::parse(uri) else {
        return false;
    };
    matches!(url.host_str(), Some("viewer.onb.ac.at"))
        || matches!(url.host_str(), Some("digital.onb.ac.at"))
            && url.path() == "/RepViewer/viewer.faces"
            && url.query_pairs().any(|(name, _)| name == "doc")
}

pub(super) fn onb_manifest(uri: &str) -> Result<Request, DiscoveryError> {
    let url = Url::parse(uri).map_err(|_| DiscoveryError::Session("invalid ONB URL".into()))?;
    let identifier = match url.host_str() {
        Some("viewer.onb.ac.at") => url
            .path_segments()
            .and_then(|mut segments| segments.next())
            .map(str::to_owned),
        Some("digital.onb.ac.at") => url
            .query_pairs()
            .find_map(|(name, value)| (name == "doc").then(|| value.into_owned())),
        _ => None,
    }
    .filter(|identifier| !identifier.is_empty())
    .ok_or_else(|| DiscoveryError::Session("missing ONB document identifier".into()))?;
    Ok(Request::new(format!(
        "https://api.onb.ac.at/iiif/presentation/v3/manifest/{identifier}"
    )))
}

pub(super) fn is_contentdm_record(uri: &str) -> bool {
    let Ok(url) = Url::parse(uri) else {
        return false;
    };
    let segments = url.path_segments().map(Iterator::collect::<Vec<_>>);
    matches!(segments.as_deref(), Some(["digital", "collection", _, "id", id, ..]) if id.parse::<u64>().is_ok())
}

pub(super) fn contentdm_metadata(uri: &str) -> Result<Request, DiscoveryError> {
    let url =
        Url::parse(uri).map_err(|_| DiscoveryError::Session("invalid CONTENTdm URL".into()))?;
    let segments = url
        .path_segments()
        .map(Iterator::collect::<Vec<_>>)
        .ok_or_else(|| DiscoveryError::Session("invalid CONTENTdm path".into()))?;
    let ["digital", "collection", collection, "id", identifier, ..] = segments.as_slice() else {
        return Err(DiscoveryError::Session(
            "invalid CONTENTdm record URL".into(),
        ));
    };
    Ok(Request::new(format!(
        "{}/digital/api/singleitem/collection/{collection}/id/{identifier}",
        url.origin().ascii_serialization()
    )))
}

pub(super) fn is_contentdm_metadata(uri: &str) -> bool {
    Url::parse(uri).is_ok_and(|url| {
        matches!(
            url.path_segments()
                .map(Iterator::collect::<Vec<_>>)
                .as_deref(),
            Some(["digital", "api", "singleitem", "collection", _, "id", _])
        )
    })
}

pub(super) fn follow_contentdm_info(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let info_uri = serde_json::from_slice::<serde_json::Value>(resource.bytes())
        .ok()
        .and_then(|value| value.get("iiifInfoUri")?.as_str().map(str::to_owned))
        .filter(|uri| !uri.is_empty())
        .ok_or_else(|| DiscoveryError::Session("CONTENTdm metadata has no IIIF URL".into()))?;
    let base = Url::parse(resource.final_uri())
        .map_err(|_| DiscoveryError::Session("invalid CONTENTdm metadata URL".into()))?;
    let origin = base.origin().ascii_serialization();
    let uri = if Url::parse(&info_uri).is_ok() {
        info_uri
    } else if info_uri.starts_with("/digital/") {
        format!("{origin}{info_uri}")
    } else if info_uri.starts_with('/') {
        format!("{origin}/digital{info_uri}")
    } else {
        format!("{origin}/digital/{info_uri}")
    };
    Ok(DiscoveryStep::Follow(Request::new(uri)))
}

pub(super) fn contains_national_gallery_image(contents: &[u8]) -> bool {
    NATIONAL_GALLERY_IMAGE.is_match(contents)
}
pub(super) fn follow_national_gallery_image(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let image = capture_utf8(
        &NATIONAL_GALLERY_IMAGE,
        resource.bytes(),
        "National Gallery IIIF image",
    )?;
    let metadata = image
        .rsplit_once("/full/")
        .map(|(service, _)| format!("{service}/info.json"))
        .ok_or_else(|| DiscoveryError::Session("invalid National Gallery IIIF image URL".into()))?;
    Ok(DiscoveryStep::Follow(Request::new(resolve_relative(
        resource.final_uri(),
        &metadata,
    ))))
}

pub(super) fn contains_london_museum_service(contents: &[u8]) -> bool {
    LONDON_MUSEUM_SERVICE.is_match(contents)
}
pub(super) fn follow_london_museum_service(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let service = capture_utf8(
        &LONDON_MUSEUM_SERVICE,
        resource.bytes(),
        "London Museum IIIF service",
    )?;
    Ok(DiscoveryStep::Follow(Request::new(format!(
        "{service}/info.json"
    ))))
}

pub(super) fn contains_philadelphia_micrio(contents: &[u8]) -> bool {
    PHILADELPHIA_MICRIO.is_match(contents)
}
pub(super) fn follow_philadelphia_micrio(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let id = capture_utf8(
        &PHILADELPHIA_MICRIO,
        resource.bytes(),
        "Philadelphia Museum Micrio ID",
    )?;
    Ok(DiscoveryStep::Follow(Request::new(format!(
        "https://i.micr.io/{id}/info.json"
    ))))
}

pub(super) fn contains_micrio_element(contents: &[u8]) -> bool {
    MICRIO_CUSTOM_ELEMENT.is_match(contents)
}
pub(super) fn follow_micrio_element(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let id = MICRIO_CUSTOM_ELEMENT
        .captures(resource.bytes())
        .and_then(|captures| captures.name("id"))
        .map(|capture| std::str::from_utf8(capture.as_bytes()))
        .transpose()
        .map_err(|_| DiscoveryError::Session("Micrio custom element ID is not UTF-8".into()))?
        .ok_or_else(|| DiscoveryError::Session("Micrio custom element lacks an ID".into()))?;
    Ok(DiscoveryStep::Follow(Request::new(format!(
        "https://i.micr.io/{id}/info.json"
    ))))
}

fn capture_utf8<'a>(
    pattern: &BytesRegex,
    contents: &'a [u8],
    label: &str,
) -> Result<&'a str, DiscoveryError> {
    pattern
        .captures(contents)
        .and_then(|captures| {
            captures
                .name("image")
                .or_else(|| captures.name("service"))
                .or_else(|| captures.name("id"))
        })
        .map(|capture| std::str::from_utf8(capture.as_bytes()))
        .transpose()
        .map_err(|_| DiscoveryError::Session(format!("{label} is not UTF-8")))?
        .ok_or_else(|| DiscoveryError::Session(format!("missing {label}")))
}
