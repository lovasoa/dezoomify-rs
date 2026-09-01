use std::sync::LazyLock;

use regex::bytes::Regex as BytesRegex;

use crate::core::{
    DiscoveryContext, DiscoveryError, DiscoveryMatch, DiscoveryResource, DiscoveryRoute,
    DiscoveryStep, Request,
};

pub(super) const ROUTE: DiscoveryRoute =
    DiscoveryMatch::ContentPredicate(contains_element).then(follow_element);

static ELEMENT: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(r#"(?is)<micr-io\b[^>]*\bid\s*=\s*["'](?P<id>[A-Za-z0-9]{5})["']"#)
        .expect("constant Micrio custom element pattern")
});

pub(super) fn contains_element(contents: &[u8]) -> bool {
    ELEMENT.is_match(contents)
}

pub(super) fn follow_element(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let id = ELEMENT
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
