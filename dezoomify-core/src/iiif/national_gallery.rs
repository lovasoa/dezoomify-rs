use crate::core::{
    DiscoveryContext, DiscoveryError, DiscoveryResource, DiscoveryStep, Request, resolve_relative,
};
use regex::bytes::Regex as BytesRegex;
use std::sync::LazyLock;
static IMAGE: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(
        r#"(?is)(?P<image>(?:https?://|/|\./|\.\./)[^"'\s<>]+\?IIIF=[^"'\s<>]+?/full/[^"'\s<>]+)"#,
    )
    .expect("constant National Gallery IIIF image pattern")
});
pub(super) fn contains_image(contents: &[u8]) -> bool {
    IMAGE.is_match(contents)
}
pub(super) fn follow_image(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let image = IMAGE
        .captures(resource.bytes())
        .and_then(|c| c.name("image"))
        .map(|c| std::str::from_utf8(c.as_bytes()))
        .transpose()
        .map_err(|_| DiscoveryError::Session("National Gallery IIIF image is not UTF-8".into()))?
        .ok_or_else(|| DiscoveryError::Session("missing National Gallery IIIF image".into()))?;
    let metadata = image
        .rsplit_once("/full/")
        .map(|(service, _)| format!("{service}/info.json"))
        .ok_or_else(|| DiscoveryError::Session("invalid National Gallery IIIF image URL".into()))?;
    Ok(DiscoveryStep::Follow(Request::new(resolve_relative(
        resource.final_uri(),
        &metadata,
    ))))
}
