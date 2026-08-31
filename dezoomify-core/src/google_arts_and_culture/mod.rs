//! Pure Google Arts & Culture two-stage discovery.

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryContext, DiscoveryError, DiscoveryMatch,
    DiscoveryResource, DiscoveryRoute, DiscoveryStep, Grid, ImageCatalog, ImageDescriptor,
    LevelDescriptor, ProcessingRecipe, Request, StableId,
};
use std::sync::Arc;
use tile_info::{PageInfo, TileInfo};
pub(crate) mod decryption;
mod tile_info;
mod url;

const ROUTES: &[DiscoveryRoute] = &[
    DiscoveryMatch::UrlSuffix("=g").then(parse_tile_information),
    DiscoveryMatch::Any.then(parse_page),
];

pub const SPEC: DezoomerSpec = DezoomerSpec::new("google_arts_and_culture", ROUTES)
    .recognizing(is_google_arts_url, "not a Google Arts & Culture URL");

fn is_google_arts_url(uri: &str) -> bool {
    uri.contains("artsandculture.google.com") || uri.contains("g.co/arts/")
}

fn parse_page(
    _context: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let source = std::str::from_utf8(resource.bytes())
        .map_err(|error| DiscoveryError::Session(error.to_string()))?;
    let page = source
        .parse::<PageInfo>()
        .map_err(|error| DiscoveryError::Session(error.to_string()))?;
    Ok(DiscoveryStep::Follow(Request::new(page.tile_info_url())))
}

fn parse_tile_information(
    context: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let page = context
        .resources()
        .map(DiscoveryResource::bytes)
        .filter_map(|bytes| std::str::from_utf8(bytes).ok())
        .find_map(|source| source.parse::<PageInfo>().ok())
        .map(Arc::new)
        .ok_or_else(|| DiscoveryError::Session("Google Arts page metadata is missing".into()))?;
    catalog(&page, resource.bytes()).map(DiscoveryStep::Complete)
}

fn catalog(page: &Arc<PageInfo>, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let TileInfo {
        tile_width,
        tile_height,
        pyramid_level,
        ..
    } = serde_xml_rs::from_reader(bytes).map_err(|error| {
        DiscoveryError::Session(format!("invalid Google Arts tile XML: {error}"))
    })?;
    let mut levels: Vec<_> = pyramid_level
        .into_iter()
        .enumerate()
        .map(|(z, level)| {
            let size = Vec2d {
                x: tile_width * level.num_tiles_x - level.empty_pels_x,
                y: tile_height * level.num_tiles_y - level.empty_pels_y,
            };
            let tile_size = Vec2d {
                x: tile_width,
                y: tile_height,
            };
            let id = StableId::new(format!("gap:{z}"));
            let request_page = Arc::clone(page);
            let source = Grid::with_processed_requests(
                id,
                size,
                tile_size,
                Vec2d::default(),
                ProcessingRecipe::GoogleArtsDecrypt,
                move |tile| {
                    let cell: Vec2d = tile.coord.into();
                    Request::new(url::compute_url(&request_page, cell.x, cell.y, z))
                },
            )
            .map_err(|error| {
                DiscoveryError::Session(format!("invalid Google Arts grid: {error}"))
            })?;
            Ok(LevelDescriptor::new(source).with_title(Some(page.name.clone())))
        })
        .collect::<Result<Vec<_>, DiscoveryError>>()?;
    levels.sort_by_key(|level| level.source.image_size().map_or(0, Vec2d::area));
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("gap:image"),
        title: Some(page.name.clone()),
        format: StableId::new("google_arts_and_culture"),
        levels,
        ..Default::default()
    })]))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{ResourceResponse, TileSource};

    fn fixture_catalog() -> ImageCatalog {
        let mut registry = crate::core::Registry::new();
        registry.register(SPEC);
        let mut operation = registry.start("https://artsandculture.google.com/asset/test");
        let page = operation.missing_resources().unwrap().pop().unwrap();
        operation
            .provide(ResourceResponse::new(
                page.id,
                include_bytes!("../../testdata/google_arts_and_culture/page_source.html"),
            ))
            .unwrap();
        let tile_info = operation.missing_resources().unwrap().pop().unwrap();
        operation
            .provide(ResourceResponse::new(
                tile_info.id,
                include_bytes!("../../testdata/google_arts_and_culture/tile_info.xml"),
            ))
            .unwrap();
        operation.finish().unwrap()
    }

    #[test]
    fn discovers_fixture_as_five_replayable_levels() {
        let mut registry = crate::core::Registry::new();
        registry.register(SPEC);
        let input = "https://artsandculture.google.com/asset/test";
        let mut operation = registry.start(input);
        let page = operation.missing_resources().unwrap().pop().unwrap();
        assert_eq!(page.request.uri, input);
        operation
            .provide(ResourceResponse::new(
                page.id,
                include_bytes!("../../testdata/google_arts_and_culture/page_source.html"),
            ))
            .unwrap();
        let tile_info = operation.missing_resources().unwrap().pop().unwrap();
        assert!(tile_info.request.uri.ends_with("=g"));
        operation
            .provide(ResourceResponse::new(
                tile_info.id,
                include_bytes!("../../testdata/google_arts_and_culture/tile_info.xml"),
            ))
            .unwrap();
        let catalog = operation.finish().unwrap();
        let [CatalogEntry::Ready(image)] = catalog.entries() else {
            panic!("Google Arts produces one ready image");
        };
        assert_eq!(image.levels.len(), 5);
        // Level names must carry the page title, as they did before the core refactor.
        assert!(
            image
                .levels
                .iter()
                .all(|level| level.title.as_deref() == Some(image.title.as_deref().unwrap_or("")))
        );
        assert!(
            image
                .levels
                .iter()
                .all(|level| level.display_label().contains("©Designers Anonymes"))
        );
        assert!(
            image
                .levels
                .windows(2)
                .all(|levels| levels[0].source.image_size().unwrap().area()
                    <= levels[1].source.image_size().unwrap().area())
        );
        let TileSource::Grid(plan) = &image.levels[0].source else {
            panic!("Google Arts geometry is a grid");
        };
        let tile = plan.tiles_row_major().next().unwrap().unwrap();
        assert_eq!(tile.processing, ProcessingRecipe::GoogleArtsDecrypt);
    }

    #[test]
    fn rejects_non_google_urls_without_requesting_data() {
        let mut registry = crate::core::Registry::new();
        registry.register(SPEC);
        let mut operation = registry.start("https://example.com/test");
        assert!(matches!(
            operation.missing_resources(),
            Err(DiscoveryError::NoCandidateAccepted { .. })
        ));
    }

    #[test]
    fn does_not_advertise_tile_metadata_without_the_required_page_context() {
        let mut registry = crate::core::Registry::new();
        registry.register(SPEC);
        let mut operation = registry.start("https://lh3.googleusercontent.com/image-id=g");
        assert!(matches!(
            operation.missing_resources(),
            Err(DiscoveryError::NoCandidateAccepted { .. })
        ));
    }

    #[test]
    fn recognizes_google_arts_short_urls() {
        let mut registry = crate::core::Registry::new();
        registry.register(SPEC);
        let mut operation = registry.start("https://g.co/arts/fixture");
        let need = operation.missing_resources().unwrap();
        assert_eq!(need.len(), 1);
        assert_eq!(need[0].request.uri, "https://g.co/arts/fixture");
    }

    #[test]
    fn catalog_preserves_page_identity_and_tile_geometry() {
        let catalog = fixture_catalog();
        let [CatalogEntry::Ready(image)] = catalog.entries() else {
            panic!("Google Arts produces one ready image");
        };
        assert_eq!(image.id, StableId::new("gap:image"));
        assert_eq!(image.format, StableId::new("google_arts_and_culture"));
        assert_eq!(image.title.as_deref(), Some("©Designers Anonymes"));

        let level = image.levels.last().expect("largest level");
        assert_eq!(level.source.image_size(), Some(Vec2d { x: 5436, y: 4080 }));
        assert_eq!(level.source.tile_size(), Some(Vec2d { x: 512, y: 512 }));
        let TileSource::Grid(plan) = &level.source else {
            panic!("Google Arts geometry is a grid");
        };
        assert_eq!(plan.count(), 88);
        let tiles: Vec<_> = plan.tiles_row_major().map(Result::unwrap).collect();
        let first = &tiles[0];
        assert_eq!(first.destination, Vec2d::default());
        assert_eq!(first.expected_size, Some(Vec2d { x: 512, y: 512 }));
        assert!(first.request.uri.contains("=x0-y0-z4-t"));
        assert_eq!(first.processing, ProcessingRecipe::GoogleArtsDecrypt);
        let last = &tiles[87];
        assert_eq!(last.destination, Vec2d { x: 5120, y: 3584 });
        assert_eq!(last.expected_size, Some(Vec2d { x: 316, y: 496 }));
        assert!(last.request.uri.contains("=x10-y7-z4-t"));
    }

    #[test]
    fn invalid_tile_information_is_reported_as_a_parser_error() {
        let mut registry = crate::core::Registry::new();
        registry.register(SPEC);
        let mut operation = registry.start("https://artsandculture.google.com/asset/test");
        let page = operation.missing_resources().unwrap().pop().unwrap();
        operation
            .provide(ResourceResponse::new(
                page.id,
                include_bytes!("../../testdata/google_arts_and_culture/page_source.html"),
            ))
            .unwrap();
        let tile_info = operation.missing_resources().unwrap().pop().unwrap();
        let error = operation
            .provide(ResourceResponse::new(
                tile_info.id,
                b"<invalid>not a tile info</invalid>",
            ))
            .unwrap_err();
        assert!(error.to_string().contains("invalid Google Arts tile XML"));
    }
}
