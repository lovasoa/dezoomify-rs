use url::Url;

use crate::core::{DiscoveryError, DiscoveryMatch, DiscoveryRoute, Request};

pub(super) const ROUTE: DiscoveryRoute = DiscoveryMatch::UrlPredicate(is_entry).map_url(manifest);

pub(super) fn prefers(uri: &str) -> bool {
    is_entry(uri)
}

pub(super) fn is_entry(uri: &str) -> bool {
    let Ok(url) = Url::parse(uri) else {
        return false;
    };
    matches!(url.host_str(), Some("viewer.onb.ac.at"))
        || matches!(url.host_str(), Some("digital.onb.ac.at"))
            && url.path() == "/RepViewer/viewer.faces"
            && url.query_pairs().any(|(name, _)| name == "doc")
}

pub(super) fn manifest(uri: &str) -> Result<Request, DiscoveryError> {
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
