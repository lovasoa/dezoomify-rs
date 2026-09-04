//! Pure discovery for `LizardTech` `ImageServer` `calcrgn` metadata.

use std::sync::{Arc, LazyLock};

use regex::Regex;
use url::Url;

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryError, DiscoveryMatch, Grid, ImageCatalog,
    ImageDescriptor, LevelDescriptor, Request, StableId,
};

static SERVER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<ImageServer\b([^>]*)>").expect("constant LizardTech server pattern")
});
static CATALOG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<Catalog\b([^>]*)>").expect("constant LizardTech catalog pattern")
});
static IMAGE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<Image\b([^>]*)/?>").expect("constant LizardTech image pattern")
});
static PARAMETER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<Parameter\b([^>]*)>(.*?)</Parameter>")
        .expect("constant LizardTech parameter pattern")
});
static ATTRIBUTE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)([A-Za-z_:][A-Za-z0-9_.:-]*)\s*=\s*[\"']([^\"']*)[\"']"#)
        .expect("constant XML attribute pattern")
});

pub const SPEC: DezoomerSpec =
    DezoomerSpec::new("lizardtech", &[DiscoveryMatch::Any.extract(catalog)])
        .recognizing(is_lizardtech_url, "not a LizardTech ImageServer URL")
        .preferring(|uri| uri.to_ascii_lowercase().contains("/lizardtech/iserv/"));

fn is_lizardtech_url(uri: &str) -> bool {
    uri.to_ascii_lowercase().contains("/lizardtech/iserv/")
}

fn catalog(url: &str, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let source_text = std::str::from_utf8(bytes)
        .map_err(|error| DiscoveryError::Session(format!("invalid LizardTech XML: {error}")))?;
    let server = SERVER_RE
        .captures(source_text)
        .and_then(|captures| captures.get(1))
        .ok_or_else(|| DiscoveryError::Session("invalid LizardTech ImageServer XML".into()))?;
    let catalog_node = CATALOG_RE
        .captures(source_text)
        .and_then(|captures| captures.get(1))
        .ok_or_else(|| DiscoveryError::Session("LizardTech XML has no Catalog".into()))?;
    let image = IMAGE_RE
        .captures(source_text)
        .and_then(|captures| captures.get(1))
        .ok_or_else(|| DiscoveryError::Session("LizardTech XML has no Image".into()))?;
    let width = attribute(image.as_str(), "width")
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| DiscoveryError::Session("missing LizardTech image width".into()))?;
    let height = attribute(image.as_str(), "height")
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| DiscoveryError::Session("missing LizardTech image height".into()))?;
    let source = Url::parse(url)
        .map_err(|_| DiscoveryError::Session("invalid LizardTech metadata URL".into()))?;
    let host = attribute(server.as_str(), "host")
        .map_or_else(|| source_host(&source).unwrap_or_default(), str::to_owned);
    if host.is_empty() {
        return Err(DiscoveryError::Session(
            "LizardTech server has no host".into(),
        ));
    }
    let path = attribute(server.as_str(), "path")
        .unwrap_or("lizardtech/iserv")
        .trim_matches('/');
    let origin: Arc<str> = format!("{}://{host}/{path}/calcrgn", source.scheme()).into();
    let catalog_name = attribute(catalog_node.as_str(), "name")
        .map(str::to_owned)
        .or_else(|| parameter(source_text, "cat"))
        .unwrap_or_default();
    let item = parameter(source_text, "item").or_else(|| {
        let parent = attribute(image.as_str(), "parent")?;
        let name = attribute(image.as_str(), "name")?;
        Some(format!("{parent}/{name}"))
    });
    let item = item
        .filter(|item| !item.is_empty())
        .ok_or_else(|| DiscoveryError::Session("LizardTech XML has no image item".into()))?;
    let title = image_title(&item);
    let levels = build_levels(width, height, &origin, &catalog_name, &item)?;
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("lizardtech:image"),
        title,
        format: StableId::new("lizardtech"),
        levels,
        ..Default::default()
    })]))
}

fn image_title(item: &str) -> Option<String> {
    let file = item.rsplit('/').next()?;
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    (!stem.is_empty()).then(|| stem.to_owned())
}

fn build_levels(
    original_width: u32,
    original_height: u32,
    origin: &Arc<str>,
    catalog: &str,
    item: &str,
) -> Result<Vec<LevelDescriptor>, DiscoveryError> {
    let mut levels = Vec::new();
    let mut width = original_width;
    let mut height = original_height;
    let mut service_level = 0_u32;
    loop {
        levels.push((width, height, service_level));
        let next_width = width.div_ceil(2);
        let next_height = height.div_ceil(2);
        if next_width.max(next_height) <= 256 {
            break;
        }
        width = next_width;
        height = next_height;
        service_level = service_level
            .checked_add(1)
            .ok_or_else(|| DiscoveryError::Session("LizardTech level overflow".into()))?;
    }
    levels.reverse();
    levels
        .into_iter()
        .enumerate()
        .map(|(ordinal, (width, height, service_level))| {
            let nominal = 512_u32
                .checked_mul(2_u32.checked_pow(u32::try_from(ordinal).unwrap_or(u32::MAX)).unwrap_or(u32::MAX))
                .ok_or_else(|| DiscoveryError::Session("LizardTech level size overflow".into()))?;
            let dx = (f64::from(nominal) - f64::from(width)) / (2.0 * f64::from(width));
            let dy = (f64::from(nominal) - f64::from(height)) / (2.0 * f64::from(height));
            let origin = Arc::clone(origin);
            let catalog = catalog.to_owned();
            let item = item.to_owned();
            let image_origin: Arc<str> = origin
                .strip_suffix("/calcrgn")
                .unwrap_or(origin.as_ref())
                .into();
            let source = Grid::with_requests(
                StableId::new(format!("lizardtech:{ordinal}")),
                Vec2d { x: width, y: height },
                Vec2d::square(512),
                Vec2d::default(),
                move |tile| {
                    let column = f64::from(tile.coord.column);
                    let row = f64::from(tile.coord.row);
                    let center_x = ((column + 0.5) * 512.0) / f64::from(width) - dx;
                    let center_y = ((row + 0.5) * 512.0) / f64::from(height) - dy;
                    Request::new(format!(
                        "{image_origin}/getimage?cat={}&item={}&wid=512&hei=512&oif=jpeg&lev={service_level}&cp={center_x},{center_y}",
                        encode_component(&catalog),
                        encode_component(&item),
                    ))
                },
            )
            .map_err(|error| DiscoveryError::Session(format!("invalid LizardTech grid: {error}")))?;
            Ok(LevelDescriptor::new(source).with_title(Some(format!(
                "LizardTech level {ordinal}"
            ))))
        })
        .collect()
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    ATTRIBUTE_RE.captures_iter(tag).find_map(|captures| {
        (captures.get(1)?.as_str().eq_ignore_ascii_case(name))
            .then(|| captures.get(2).expect("attribute value capture").as_str())
    })
}

fn parameter(xml: &str, name: &str) -> Option<String> {
    PARAMETER_RE.captures_iter(xml).find_map(|captures| {
        let attributes = captures.get(1)?.as_str();
        if !attribute(attributes, "name")?.eq_ignore_ascii_case(name) {
            return None;
        }
        Some(captures.get(2)?.as_str().trim().to_owned())
    })
}

fn encode_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
                vec![byte as char]
            } else {
                format!("%{byte:02X}").chars().collect()
            }
        })
        .collect()
}

fn source_host(source: &Url) -> Option<String> {
    let host = source.host_str()?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    Some(match source.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}
