use crate::core::{DiscoveryContext, DiscoveryError, DiscoveryResource, DiscoveryStep, Request};
use regex::bytes::Regex as BytesRegex;
use std::sync::LazyLock;
static MICRIO: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(r#"(?is)(?:philamuseum|philadelphia museum).*?\\?"shortId\\?"\s*:\s*\\?"(?P<id>[A-Za-z0-9_-]{3,32})\\?""#).expect("constant Philadelphia Museum Micrio pattern")
});
pub(super) fn contains_micrio(contents: &[u8]) -> bool {
    MICRIO.is_match(contents)
}
pub(super) fn follow_micrio(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let id = MICRIO
        .captures(resource.bytes())
        .and_then(|c| c.name("id"))
        .map(|c| std::str::from_utf8(c.as_bytes()))
        .transpose()
        .map_err(|_| DiscoveryError::Session("Philadelphia Museum Micrio ID is not UTF-8".into()))?
        .ok_or_else(|| DiscoveryError::Session("missing Philadelphia Museum Micrio ID".into()))?;
    Ok(DiscoveryStep::Follow(Request::new(format!(
        "https://i.micr.io/{id}/info.json"
    ))))
}
