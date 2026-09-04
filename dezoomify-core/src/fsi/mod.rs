//! Pure discovery for FSI Server (Neptune Labs) images.

use std::sync::LazyLock;

use regex::Regex;

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryContext, DiscoveryError, DiscoveryMatch,
    DiscoveryResource, DiscoveryRoute, DiscoveryStep, Grid, ImageCatalog, ImageDescriptor,
    LevelDescriptor, Request, StableId, resolve_relative,
};

static SOURCE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[?&])source=([^&#]+)").expect("constant FSI source pattern")
});
static SERVER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)([^\s\"']*/server[^\s\"']*)"#).expect("constant FSI server pattern")
});
static WIDTH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bwidth\s+value\s*=\s*[\"']?(\d+)"#).expect("constant FSI width pattern")
});
static HEIGHT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bheight\s+value\s*=\s*[\"']?(\d+)"#).expect("constant FSI height pattern")
});

const ROUTES: &[DiscoveryRoute] = &[
    DiscoveryMatch::UrlPredicate(is_server_url).map_url(metadata_url),
    DiscoveryMatch::ContentPredicate(contains_server).then(follow_page_server),
    DiscoveryMatch::Any.extract(catalog),
];

pub const SPEC: DezoomerSpec = DezoomerSpec::new("fsi", ROUTES).preferring(is_server_url);

fn is_server_url(uri: &str) -> bool {
    uri.split_once('?').is_some_and(|(path, query)| {
        path.trim_end_matches('/').ends_with("/server") && SOURCE_RE.is_match(query)
    })
}

fn metadata_url(uri: &str) -> Result<Request, DiscoveryError> {
    let source = SOURCE_RE
        .captures(uri)
        .and_then(|captures| captures.get(1))
        .ok_or_else(|| DiscoveryError::Session("FSI URL has no source parameter".into()))?;
    let origin = uri.split_once('?').map_or(uri, |(origin, _)| origin);
    Ok(Request::new(format!(
        "{origin}?type=info&source={}",
        source.as_str()
    )))
}

fn contains_server(bytes: &[u8]) -> bool {
    SERVER_RE.is_match(&String::from_utf8_lossy(bytes))
}

fn follow_page_server(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    // Server URLs are embedded in HTML, where query separators are escaped.
    let page = resource.text_lossy().replace("&amp;", "&");
    let server = SERVER_RE
        .captures_iter(&page)
        .find_map(|captures| {
            let server = captures.get(1)?.as_str();
            SOURCE_RE.is_match(server).then_some(server.to_owned())
        })
        .ok_or_else(|| DiscoveryError::Session("no FSI URL found in page".into()))?;
    let server = resolve_relative(resource.final_uri(), &server);
    metadata_url(&server).map(DiscoveryStep::Follow)
}

fn catalog(url: &str, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let width = number(&WIDTH_RE, bytes, "width")?;
    let height = number(&HEIGHT_RE, bytes, "height")?;
    let source = SOURCE_RE
        .captures(url)
        .and_then(|captures| captures.get(1))
        .ok_or_else(|| DiscoveryError::Session("FSI metadata URL has no source".into()))?
        .as_str()
        .to_owned();
    let origin = url
        .split_once('?')
        .map_or(url, |(origin, _)| origin)
        .to_owned();
    let title = image_title(&source);
    let source = Grid::with_requests(
        StableId::new("fsi:level"),
        Vec2d {
            x: width,
            y: height,
        },
        Vec2d::square(512),
        Vec2d::default(),
        move |tile| {
            let position = Vec2d {
                x: tile.coord.column * 512,
                y: tile.coord.row * 512,
            };
            let size = Vec2d {
                x: 512.min(width - position.x),
                y: 512.min(height - position.y),
            };
            Request::new(format!(
                "{origin}?type=image&source={source}&width={}&height={}&rect={},{},{},{}",
                size.x,
                size.y,
                ratio(position.x, width),
                ratio(position.y, height),
                ratio(size.x, width),
                ratio(size.y, height),
            ))
        },
    )
    .map_err(|error| DiscoveryError::Session(format!("invalid FSI grid: {error}")))?;
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("fsi:image"),
        title,
        format: StableId::new("fsi"),
        levels: vec![LevelDescriptor::new(source)],
        ..Default::default()
    })]))
}

fn number(regex: &Regex, bytes: &[u8], name: &str) -> Result<u32, DiscoveryError> {
    regex
        .captures(&String::from_utf8_lossy(bytes))
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse().ok())
        .filter(|number| *number > 0)
        .ok_or_else(|| DiscoveryError::Session(format!("FSI metadata has invalid {name}")))
}

fn image_title(source: &str) -> Option<String> {
    let file = source.rsplit('/').next()?;
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    (!stem.is_empty()).then(|| stem.to_owned())
}

fn ratio(numerator: u32, denominator: u32) -> f64 {
    f64::from(numerator) / f64::from(denominator)
}
