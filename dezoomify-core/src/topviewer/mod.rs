//! Pure discovery for Picturae Memorix/TopViewer images.

use std::sync::{Arc, LazyLock};

use regex::Regex;
use serde_json::Value;
use url::Url;

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryContext, DiscoveryError, DiscoveryMatch,
    DiscoveryResource, DiscoveryRoute, DiscoveryStep, Grid, ImageCatalog, ImageDescriptor,
    LevelDescriptor, Request, StableId, resolve_relative,
};
use crate::web_page::decode_html_entities;

static THUMBNAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)images\.memorix\.nl/([^/]+)/thumb/[^/]+/(.*?)\.jpg")
        .expect("constant TopViewer thumbnail pattern")
});
static MEDIABANK_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<pic-mediabank\b[^>]*>").expect("constant TopViewer mediabank pattern")
});
static API_KEY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bdata-api-key\s*=\s*[\"']([^\"']+)[\"']"#)
        .expect("constant TopViewer API key pattern")
});
static API_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bdata-api-url\s*=\s*[\"']([^\"']+)[\"']"#)
        .expect("constant TopViewer API URL pattern")
});
static ENTITIES_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bdata-entities\s*=\s*[\"']([^\"']+)[\"']"#)
        .expect("constant TopViewer entities pattern")
});
static DETAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)/detail/([a-z0-9-]+)/media/([a-z0-9-]+)")
        .expect("constant TopViewer detail pattern")
});

const ROUTES: &[DiscoveryRoute] = &[
    DiscoveryMatch::ContentPredicate(contains_mediabank).then(follow_mediabank),
    DiscoveryMatch::ContentPredicate(contains_thumbnail).then(follow_thumbnail),
    DiscoveryMatch::UrlPredicate(is_media_api).then(follow_media),
    DiscoveryMatch::ContentPredicate(contains_topviews).extract(catalog),
];

pub const SPEC: DezoomerSpec = DezoomerSpec::new("topviewer", ROUTES)
    .preferring(|uri| uri.contains("topviewjson") || uri.contains("memorix"));

fn contains_topviews(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes).contains("\"topviews\"")
}

fn contains_thumbnail(bytes: &[u8]) -> bool {
    THUMBNAIL_RE.is_match(&String::from_utf8_lossy(bytes))
}

fn contains_mediabank(bytes: &[u8]) -> bool {
    MEDIABANK_TAG_RE.is_match(&String::from_utf8_lossy(bytes))
}

fn follow_thumbnail(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let page = resource.text_lossy();
    let captures = THUMBNAIL_RE
        .captures(&page)
        .ok_or_else(|| DiscoveryError::Session("unable to find a Memorix thumbnail".into()))?;
    let server = captures
        .get(1)
        .map(|value| value.as_str().to_owned())
        .ok_or_else(|| DiscoveryError::Session("thumbnail has no image server".into()))?;
    let image = captures
        .get(2)
        .map(|value| value.as_str().to_owned())
        .ok_or_else(|| DiscoveryError::Session("thumbnail has no image ID".into()))?;
    Ok(DiscoveryStep::Follow(Request::new(format!(
        "https://images.memorix.nl/{server}/topviewjson/memorix/{image}"
    ))))
}

fn follow_mediabank(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let page = resource.text_lossy();
    let tag = MEDIABANK_TAG_RE
        .find(&page)
        .map(|match_| match_.as_str())
        .ok_or_else(|| DiscoveryError::Session("TopViewer page has no media element".into()))?;
    let api_key = capture_attribute(&API_KEY_RE, tag, "API key")?;
    let api_reference = capture_attribute(&API_URL_RE, tag, "API URL")?;
    let entities = ENTITIES_RE
        .captures(tag)
        .and_then(|captures| captures.get(1))
        .map(|value| decode_html_entities(value.as_str()));
    let page = Url::parse(resource.final_uri())
        .map_err(|_| DiscoveryError::Session("invalid TopViewer page URL".into()))?;
    let detail = DETAIL_RE
        .captures(resource.final_uri())
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_owned());
    let mut api = page
        .join(&api_reference)
        .map_err(|_| DiscoveryError::Session("invalid TopViewer API URL".into()))?;
    let mut path = api.path().trim_end_matches('/').to_owned();
    path.push_str("/media");
    if let Some(detail) = &detail {
        path.push('/');
        path.push_str(detail);
    }
    api.set_path(&path);

    let mut parameters: Vec<(String, String)> = api
        .query_pairs()
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .filter(|(name, _)| !matches!(name.as_str(), "apiKey" | "entities[0]" | "rows"))
        .collect();
    if detail.is_none() {
        for name in ["q", "page", "fq[]", "sort"] {
            parameters.extend(
                page.query_pairs()
                    .filter(|(candidate, _)| candidate == name)
                    .map(|(name, value)| (name.into_owned(), value.into_owned())),
            );
        }
        parameters.push(("rows".into(), "1".into()));
    }
    parameters.push(("apiKey".into(), api_key));
    if let Some(entities) = entities {
        parameters.push(("entities[0]".into(), entities));
    }
    api.set_query(None);
    {
        let mut query = api.query_pairs_mut();
        for (name, value) in parameters {
            query.append_pair(&name, &value);
        }
    }
    Ok(DiscoveryStep::Follow(Request::new(api.to_string())))
}

fn capture_attribute(regex: &Regex, tag: &str, label: &str) -> Result<String, DiscoveryError> {
    regex
        .captures(tag)
        .and_then(|captures| captures.get(1))
        .map(|value| decode_html_entities(value.as_str()))
        .ok_or_else(|| DiscoveryError::Session(format!("TopViewer element has no {label}")))
}

fn is_media_api(uri: &str) -> bool {
    Url::parse(uri).is_ok_and(|url| {
        let path = url.path().trim_end_matches('/');
        path.ends_with("/media") || (path.contains("/media/") && url.query().is_some())
    })
}

fn follow_media(
    context: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let value: Value = serde_json::from_slice(resource.bytes()).map_err(|error| {
        DiscoveryError::Session(format!("unable to parse TopViewer media response: {error}"))
    })?;
    let wanted = context.resources().rev().find_map(|page| {
        DETAIL_RE
            .captures(page.final_uri())
            .and_then(|captures| captures.get(2))
            .map(|value| value.as_str().to_owned())
    });
    let asset = value
        .get("media")
        .and_then(Value::as_array)
        .and_then(|media| media.first())
        .and_then(|media| media.get("asset"))
        .and_then(Value::as_array)
        .and_then(|assets| {
            assets.iter().find(|asset| {
                wanted
                    .as_deref()
                    .is_none_or(|wanted| asset.get("uuid").and_then(Value::as_str) == Some(wanted))
                    && asset.get("topview").and_then(Value::as_str).is_some()
            })
        })
        .and_then(|asset| asset.get("topview"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            DiscoveryError::Session("no zoomable image found in TopViewer response".into())
        })?;
    Ok(DiscoveryStep::Follow(Request::new(resolve_relative(
        resource.final_uri(),
        asset,
    ))))
}

fn catalog(url: &str, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        DiscoveryError::Session(format!("unable to parse TopViewer metadata: {error}"))
    })?;
    let view = value
        .get("topviews")
        .and_then(Value::as_array)
        .and_then(|views| views.first())
        .ok_or_else(|| DiscoveryError::Session("TopViewer metadata has no topviews".into()))?;
    let config = value
        .get("config")
        .and_then(Value::as_object)
        .and_then(|config| config.get("tileurl_v2"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            DiscoveryError::Session("TopViewer metadata has no tile URL template".into())
        })?;
    let width = number(view, "width")?;
    let height = number(view, "height")?;
    let tile_size = number(view, "tileWidth")?;
    let layers = view
        .get("layers")
        .and_then(Value::as_array)
        .ok_or_else(|| DiscoveryError::Session("TopViewer metadata has no layers".into()))?;
    let layer = layers
        .iter()
        .max_by_key(|layer| layer.get("width").and_then(Value::as_u64).unwrap_or(0))
        .ok_or_else(|| DiscoveryError::Session("TopViewer metadata has no usable layer".into()))?;
    let first_tile = number(layer, "starttile")?;
    let columns = number(layer, "cols")?;
    let filepath = view.get("filepath").and_then(Value::as_str);
    let template = resolve_template(url, config)
        .replace("{file}", filepath.unwrap_or("image"))
        .replace("{extension}", "jpg");
    let template: Arc<str> = template.into();
    let source = Grid::with_requests(
        StableId::new("topviewer:level"),
        Vec2d {
            x: width,
            y: height,
        },
        Vec2d::square(tile_size),
        Vec2d::default(),
        move |tile| {
            let tile_number = u64::from(first_tile)
                + u64::from(tile.coord.column)
                + u64::from(tile.coord.row) * u64::from(columns);
            Request::new(template.replace("{tile}", &tile_number.to_string()))
        },
    )
    .map_err(|error| DiscoveryError::Session(format!("invalid TopViewer grid: {error}")))?;
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("topviewer:image"),
        title: filepath.and_then(image_title),
        format: StableId::new("topviewer"),
        levels: vec![LevelDescriptor::new(source)],
        ..Default::default()
    })]))
}

fn image_title(filepath: &str) -> Option<String> {
    let file = filepath.rsplit(['/', '\\']).next()?;
    (!file.is_empty()).then(|| file.to_owned())
}

fn resolve_template(base: &str, template: &str) -> String {
    resolve_relative(base, &template.replace('{', "%7B").replace('}', "%7D"))
        .replace("%7B", "{")
        .replace("%7b", "{")
        .replace("%7D", "}")
        .replace("%7d", "}")
}

fn number(value: &Value, name: &str) -> Result<u32, DiscoveryError> {
    value
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
        .filter(|number| *number > 0)
        .ok_or_else(|| DiscoveryError::Session(format!("TopViewer metadata has invalid {name}")))
}
