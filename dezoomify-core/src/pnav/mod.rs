//! Pure discovery for the crop-based pnav image service.

use std::sync::{Arc, LazyLock};

use regex::Regex;
use serde::Deserialize;

use crate::Vec2d;
use crate::core::{
    AdaptiveProgram, AdaptiveSource, CatalogEntry, DezoomerSpec, DiscoverableStep,
    DiscoveryContext, DiscoveryError, DiscoveryMatch, DiscoveryResource, DiscoveryRoute,
    DiscoveryStep, Grid, ImageCatalog, ImageDescriptor, LevelDescriptor, ObservationResult,
    ProbeContinuation, Request, StableId, TileId, TileRole, TileSourceError, TileSpec,
    resolve_relative,
};
use crate::web_page::page_title;

const TILE_SIZE: u32 = 512;
static META_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<meta\b[^>]*>").expect("constant pnav meta tag pattern"));
static ATTRIBUTE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)([A-Za-z_:][A-Za-z0-9_.:-]*)\s*=\s*[\"']([^\"']*)[\"']"#)
        .expect("constant pnav attribute pattern")
});

const ROUTES: &[DiscoveryRoute] = &[
    DiscoveryMatch::ContentPredicate(contains_image_meta).then(follow_image_json),
    DiscoveryMatch::UrlPredicate(is_image_json).then(complete_from_json),
];

pub const SPEC: DezoomerSpec = DezoomerSpec::new("pnav", ROUTES)
    .recognizing(is_pnav_url, "not a pnav entity URL")
    .preferring(is_pnav_url);

fn is_pnav_url(uri: &str) -> bool {
    let path = uri.split_once(['?', '#']).map_or(uri, |(path, _)| path);
    let path = path.trim_end_matches('/');
    let mut segments = path.rsplit('/');
    segments
        .next()
        .is_some_and(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        && segments
            .next()
            .is_some_and(|value| value.eq_ignore_ascii_case("OBJECT"))
        && segments
            .next()
            .is_some_and(|value| value.eq_ignore_ascii_case("entity"))
}

fn is_image_json(uri: &str) -> bool {
    uri.split_once(['?', '#'])
        .map_or(uri, |(path, _)| path)
        .to_ascii_lowercase()
        .ends_with(".json")
}

fn contains_image_meta(bytes: &[u8]) -> bool {
    extract_image_url(&String::from_utf8_lossy(bytes), "").is_some()
}

fn extract_image_url(page: &str, page_uri: &str) -> Option<String> {
    META_RE.captures_iter(page).find_map(|captures| {
        let tag = captures.get(0)?.as_str();
        let property = attribute(tag, "property")?;
        if !property.eq_ignore_ascii_case("og:image") {
            return None;
        }
        let content = attribute(tag, "content")?;
        let image = content.split_once('?').map_or(content, |(image, _)| image);
        let image = image.replace("&amp;", "&");
        if image.is_empty() {
            None
        } else if page_uri.is_empty() {
            Some(image)
        } else {
            Some(resolve_relative(page_uri, &image))
        }
    })
}

fn follow_image_json(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let image = extract_image_url(&resource.text_lossy(), resource.final_uri())
        .ok_or_else(|| DiscoveryError::Session("pnav page has no og:image URL".into()))?;
    Ok(DiscoveryStep::Follow(Request::new(json_url(&image)?)))
}

fn complete_from_json(
    context: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let metadata: Metadata = serde_json::from_slice(resource.bytes()).map_err(|error| {
        DiscoveryError::Session(format!("unable to parse pnav image metadata: {error}"))
    })?;
    if metadata.width == 0 || metadata.height == 0 {
        return Err(DiscoveryError::Session(
            "pnav image dimensions must be positive".into(),
        ));
    }
    let page = context
        .resources()
        .rev()
        .find(|page| extract_image_url(&page.text_lossy(), page.final_uri()).is_some())
        .ok_or_else(|| {
            DiscoveryError::Session("pnav page is missing from discovery history".into())
        })?;
    let page_text = page.text_lossy();
    let image = extract_image_url(&page_text, page.final_uri()).ok_or_else(|| {
        DiscoveryError::Session("pnav page is missing from discovery history".into())
    })?;
    let title = page_title(&page_text);
    let source = AdaptiveSource::new(
        StableId::new("pnav:level"),
        PnavProgram {
            image_url: image,
            width: metadata.width,
            height: metadata.height,
        },
    );
    Ok(DiscoveryStep::Complete(ImageCatalog::new([
        CatalogEntry::Ready(ImageDescriptor {
            id: StableId::new("pnav:image"),
            title,
            format: StableId::new("pnav"),
            levels: vec![LevelDescriptor::new(source)],
            ..Default::default()
        }),
    ])))
}

fn json_url(image: &str) -> Result<String, DiscoveryError> {
    let path_end = image.find(['?', '#']).unwrap_or(image.len());
    let (path, suffix) = image.split_at(path_end);
    let dot = path
        .rfind('.')
        .filter(|index| {
            path[*index..].eq_ignore_ascii_case(".jpg")
                || path[*index..].eq_ignore_ascii_case(".jpeg")
        })
        .ok_or_else(|| DiscoveryError::Session("pnav image URL has no JPEG extension".into()))?;
    Ok(format!("{}.json{suffix}", &path[..dot]))
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    ATTRIBUTE_RE.captures_iter(tag).find_map(|captures| {
        (captures.get(1)?.as_str().eq_ignore_ascii_case(name))
            .then(|| captures.get(2).expect("attribute value capture").as_str())
    })
}

#[derive(Clone, Debug)]
struct PnavProgram {
    image_url: String,
    width: u32,
    height: u32,
}

impl AdaptiveProgram for PnavProgram {
    fn start(&self) -> DiscoverableStep {
        let program = self.clone();
        let image_url = self.image_url.clone();
        let tile = TileSpec {
            id: TileId::new(StableId::new("pnav:level"), 0),
            request: Request::new(format!(
                "{image_url}?w=2000&h=2000&cl=0&ct=0&cw={TILE_SIZE}&ch={TILE_SIZE}"
            )),
            destination: Vec2d::default(),
            expected_size: None,
            processing: dezoomify_processing_none(),
            role: TileRole::ProbeAndOutput,
        };
        DiscoverableStep::Probe {
            tile,
            continuation: ProbeContinuation::new(move |result| program.resolve(result)),
        }
    }
}

impl PnavProgram {
    fn resolve(self, result: ObservationResult) -> Result<DiscoverableStep, TileSourceError> {
        let ObservationResult::Available { size } = result else {
            return Ok(DiscoverableStep::Empty);
        };
        if size.x == 0 || size.y == 0 || size == Vec2d::square(1) {
            return Ok(DiscoverableStep::Empty);
        }
        let image_size = Vec2d {
            x: rounded_scale(self.width, size.x)?,
            y: rounded_scale(self.height, size.y)?,
        };
        let tile_width = size.x;
        let tile_height = size.y;
        let width = self.width;
        let height = self.height;
        let image_url: Arc<str> = self.image_url.into();
        let source = Grid::with_requests(
            StableId::new("pnav:level"),
            image_size,
            size,
            Vec2d::default(),
            move |tile| {
                let column = tile.coord.column;
                let row = tile.coord.row;
                let crop_left = column * TILE_SIZE;
                let crop_top = row * TILE_SIZE;
                let crop_width = TILE_SIZE.min(width - crop_left);
                let crop_height = TILE_SIZE.min(height - crop_top);
                let last_column = !width.is_multiple_of(TILE_SIZE) && column == width / TILE_SIZE;
                let last_row = !height.is_multiple_of(TILE_SIZE) && row == height / TILE_SIZE;
                let output_width = if last_column {
                    scaled_ceil(crop_width, tile_width)
                } else {
                    tile_width
                };
                let output_height = if last_row {
                    scaled_ceil(crop_height, tile_height)
                } else {
                    tile_height
                };
                Request::new(format!(
                    "{image_url}?w={output_width}&h={output_height}&cl={crop_left}&ct={crop_top}&cw={crop_width}&ch={crop_height}"
                ))
            },
        )?;
        Ok(DiscoverableStep::Resolved {
            grid: source,
            previously_output: vec![Vec2d::default()],
        })
    }
}

fn rounded_scale(value: u32, natural_size: u32) -> Result<u32, TileSourceError> {
    let numerator = u64::from(value)
        .checked_mul(u64::from(natural_size))
        .ok_or(TileSourceError::ArithmeticOverflow)?;
    u32::try_from((numerator + u64::from(TILE_SIZE / 2)) / u64::from(TILE_SIZE))
        .map_err(|_| TileSourceError::ArithmeticOverflow)
}

fn scaled_ceil(value: u32, natural_size: u32) -> u32 {
    value.saturating_mul(natural_size).div_ceil(TILE_SIZE)
}

fn dezoomify_processing_none() -> crate::core::ProcessingRecipe {
    crate::core::ProcessingRecipe::None
}

#[derive(Debug, Deserialize)]
struct Metadata {
    width: u32,
    height: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_crop_dimensions_are_scaled_per_axis() {
        let program = PnavProgram {
            image_url: "https://example.test/image.jpg".into(),
            width: 600,
            height: 700,
        };
        let DiscoverableStep::Resolved { grid, .. } = program
            .resolve(ObservationResult::Available {
                size: Vec2d::square(512),
            })
            .unwrap()
        else {
            panic!("pnav program must resolve")
        };
        let requests: Vec<_> = grid
            .tiles_row_major()
            .map(|tile| tile.unwrap().request.uri)
            .collect();
        assert!(
            requests
                .iter()
                .any(|request| request.contains("w=88&h=512"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.contains("w=512&h=188"))
        );
    }
}
