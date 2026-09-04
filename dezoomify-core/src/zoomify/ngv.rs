use std::sync::LazyLock;

use regex::bytes::Regex as BytesRegex;

use crate::core::{
    DiscoveryContext, DiscoveryError, DiscoveryMatch, DiscoveryResource, DiscoveryRoute,
    DiscoveryStep, Request, resolve_relative,
};

use super::{append_path_component, capture_text};

static IMAGE_PATH: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(r#"(?is)\bvar\s+url\s*=\s*['"](?P<image>[^'"]+)['"]"#)
        .expect("constant NGV Zoomify path pattern")
});

pub(super) const ROUTE: DiscoveryRoute =
    DiscoveryMatch::UrlPredicate(is_work_page).then(follow_image_path);

pub(super) fn prefers(uri: &str) -> bool {
    is_work_page(uri)
}

pub(super) fn is_work_page(uri: &str) -> bool {
    uri.contains("ngv.vic.gov.au/explore/collection/work")
}

pub(super) fn follow_image_path(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let path = IMAGE_PATH
        .captures(resource.bytes())
        .and_then(|captures| capture_text(&captures, "image"))
        .ok_or_else(|| {
            DiscoveryError::Session("NGV page does not declare a Zoomify path".into())
        })?;
    let image_uri = resolve_relative(resource.final_uri(), &path);
    Ok(DiscoveryStep::Follow(Request::new(append_path_component(
        &image_uri,
        "ImageProperties.xml",
    ))))
}
