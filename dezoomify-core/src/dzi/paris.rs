use std::sync::LazyLock;

use regex::bytes::Regex as BytesRegex;

use crate::core::{
    DiscoveryContext, DiscoveryError, DiscoveryMatch, DiscoveryResource, DiscoveryRoute,
    DiscoveryStep, Request, resolve_relative,
};

static DEEPZOOM_MANIFEST: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(
        r#"(?is)\bdeepZoomManifest\b["']?\s*[:=]\s*["'](?P<metadata>[^"']+\.(?:dzi|xml))["']"#,
    )
    .expect("constant Paris Deep Zoom manifest pattern")
});

pub(super) const ARK_ROUTE: DiscoveryRoute = DiscoveryMatch::UrlPredicate(is_ark).map_url(reader);
pub(super) const MANIFEST_ROUTE: DiscoveryRoute =
    DiscoveryMatch::ContentPredicate(contains_manifest).then(follow_manifest);

pub(super) fn prefers(uri: &str) -> bool {
    is_ark(uri)
}

pub(super) fn is_ark(uri: &str) -> bool {
    uri.starts_with("https://bibliotheques-specialisees.paris.fr/ark:/")
}

pub(super) fn reader(uri: &str) -> Result<Request, DiscoveryError> {
    let ark = uri
        .strip_prefix("https://bibliotheques-specialisees.paris.fr/ark:")
        .filter(|ark| ark.split('/').filter(|part| !part.is_empty()).count() >= 3)
        .ok_or_else(|| DiscoveryError::Session("invalid Paris ARK URL".into()))?;
    let mut parts = ark.split('/').filter(|part| !part.is_empty());
    let prefix = format!(
        "/{}/{}/{}",
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default()
    );
    Ok(Request::new(format!(
        "https://bibliotheques-specialisees.paris.fr/in/imageReader.xhtml?id=ark:{prefix}&updateUrl=updateUrl1653&ark={ark}&selectedTab=otherdocs"
    )))
}

pub(super) fn contains_manifest(contents: &[u8]) -> bool {
    DEEPZOOM_MANIFEST.is_match(contents)
}

pub(super) fn follow_manifest(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let metadata = DEEPZOOM_MANIFEST
        .captures(resource.bytes())
        .and_then(|captures| captures.name("metadata"))
        .map(|capture| std::str::from_utf8(capture.as_bytes()))
        .transpose()
        .map_err(|_| DiscoveryError::Session("Paris Deep Zoom manifest URL is not UTF-8".into()))?
        .ok_or_else(|| {
            DiscoveryError::Session("Paris page lacks a Deep Zoom manifest URL".into())
        })?;
    Ok(DiscoveryStep::Follow(Request::new(resolve_relative(
        resource.final_uri(),
        metadata,
    ))))
}
