//! Pure discovery for Zoomify `ImageProperties.xml` pyramids.

use std::sync::{Arc, LazyLock};

use image_properties::ImageProperties;
use regex::Regex;

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryError, Grid, ImageCatalog, ImageDescriptor,
    LevelDescriptor, Request, ResourceRequest, StableId,
};

mod image_properties;

pub const SPEC: DezoomerSpec = DezoomerSpec::routed("zoomify", metadata_request, load_catalog)
    .recognizing(is_zoomify_url, "not a Zoomify metadata or tile URL")
    .preferring(is_zoomify_url);

static TILE_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|/)TileGroup\d+/\d+-\d+-\d+\.jpe?g(?:[?#].*)?$")
        .expect("constant Zoomify tile URL pattern")
});

fn is_tile_url(uri: &str) -> bool {
    TILE_URL_RE.is_match(uri)
}

fn is_zoomify_url(uri: &str) -> bool {
    uri.contains("/ImageProperties.xml") || is_tile_url(uri)
}

#[allow(clippy::unnecessary_wraps)]
fn metadata_request(input: &str) -> Result<ResourceRequest, DiscoveryError> {
    let uri = TILE_URL_RE.find(input).map_or_else(
        || input.to_owned(),
        |tile| {
            let prefix_end = if input.as_bytes()[tile.start()] == b'/' {
                tile.start() + 1
            } else {
                tile.start()
            };
            let prefix = &input[..prefix_end];
            let metadata = if prefix.is_empty() || prefix.ends_with('/') {
                format!("{prefix}ImageProperties.xml")
            } else {
                format!("{prefix}/ImageProperties.xml")
            };
            let suffix = tile
                .as_str()
                .find(['?', '#'])
                .map_or("", |index| &tile.as_str()[index..]);
            format!("{metadata}{suffix}")
        },
    );
    Ok(ResourceRequest::new(uri))
}

fn load_catalog(url: &str, contents: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let properties: ImageProperties = serde_xml_rs::from_reader(contents).map_err(|error| {
        DiscoveryError::Session(format!("unable to parse Zoomify XML: {error}"))
    })?;
    let base_url: Arc<str> = url
        .split("/ImageProperties.xml")
        .next()
        .unwrap_or(url)
        .into();
    let base_name = base_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("Zoomify");
    let full_resolution_only = properties.is_full_resolution_only();
    let (level_info, warnings) = properties.levels_with_warnings();
    let levels = level_info
        .into_iter()
        .enumerate()
        .map(|(index, info)| {
            let size = info.size;
            let tile_size = info.tile_size;
            let base_url = Arc::clone(&base_url);
            let source = Grid::with_requests(
                format!("zoomify:{index}").into(),
                size,
                tile_size,
                Vec2d::default(),
                move |tile| {
                    let cell: Vec2d = tile.coord.into();
                    let tile_group = if full_resolution_only {
                        0
                    } else {
                        (u64::from(info.tiles_before) + tile.row_major_ordinal) / 256
                    };
                    Request::new(format!(
                        "{base_url}/TileGroup{tile_group}/{index}-{}-{}.jpg",
                        cell.x, cell.y
                    ))
                },
            )
            .map_err(|error| DiscoveryError::Session(format!("invalid Zoomify grid: {error}")))?;
            Ok(LevelDescriptor::new(source).with_title(Some(format!(
                "{base_name} Zoomify level {index} ({: >5}×{: >5} pixels)",
                size.x, size.y,
            ))))
        })
        .collect::<Result<Vec<_>, DiscoveryError>>()?;
    let title = base_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("zoomify:image"),
        title,
        format: StableId::new("zoomify"),
        levels,
        warnings,
    })]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::TileSource;

    #[test]
    fn tile_urls_request_sibling_metadata() {
        let mut registry = crate::core::Registry::new();
        registry.register(SPEC);
        let mut operation =
            registry.start("https://example.com/images/book/TileGroup0/3-0-0.jpg?token=secret");
        let need = operation.missing_resources().unwrap().pop().unwrap();
        assert_eq!(
            need.request.uri,
            "https://example.com/images/book/ImageProperties.xml?token=secret"
        );
        operation
            .provide(crate::core::ResourceResponse {
                id: need.id,
                bytes: br#"<IMAGE_PROPERTIES WIDTH="512" HEIGHT="256" NUMTILES="2" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#.to_vec(),
            })
            .unwrap();
        let catalog = operation.finish().unwrap();
        let CatalogEntry::Ready(image) = &catalog.entries()[0] else {
            panic!("Zoomify metadata must produce a ready image")
        };
        let TileSource::Grid(plan) = &image.levels[0].source else {
            panic!("Zoomify levels must be grids")
        };
        assert_eq!(
            plan.tiles_row_major().next().unwrap().unwrap().request.uri,
            "https://example.com/images/book/TileGroup0/0-0-0.jpg"
        );
    }

    fn ready_image(url: &str, contents: &[u8]) -> ImageDescriptor {
        match load_catalog(url, contents)
            .unwrap()
            .into_entries()
            .pop()
            .unwrap()
        {
            CatalogEntry::Ready(image) => image,
            CatalogEntry::Deferred(_) => panic!("Zoomify is resolved"),
        }
    }

    fn levels(url: &str, contents: &[u8]) -> Vec<LevelDescriptor> {
        ready_image(url, contents).levels
    }

    #[test]
    fn panorama_preserves_tile_order_and_normalized_levels() {
        let contents = br#"<IMAGE_PROPERTIES WIDTH="174550" HEIGHT="16991" NUMTILES="61284" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#;
        let levels = levels("http://x.fr/y/ImageProperties.xml?t", contents);
        assert_eq!(levels.len(), 11);
        assert!(
            levels
                .windows(2)
                .all(|pair| pair[0].source.image_size().unwrap().area()
                    <= pair[1].source.image_size().unwrap().area())
        );
        let TileSource::Grid(plan) = &levels[3].source else {
            unreachable!()
        };
        let urls: Vec<_> = plan
            .tiles_row_major()
            .take(6)
            .map(Result::unwrap)
            .map(|tile| tile.request.uri)
            .collect();
        assert_eq!(
            urls,
            vec![
                "http://x.fr/y/TileGroup0/3-0-0.jpg",
                "http://x.fr/y/TileGroup0/3-1-0.jpg",
                "http://x.fr/y/TileGroup0/3-2-0.jpg",
                "http://x.fr/y/TileGroup0/3-3-0.jpg",
                "http://x.fr/y/TileGroup0/3-4-0.jpg",
                "http://x.fr/y/TileGroup0/3-5-0.jpg"
            ]
        );
    }

    #[test]
    fn titles_tile_groups_and_warnings_are_retained() {
        let contents = br#"<IMAGE_PROPERTIES WIDTH="12000" HEIGHT="9788" NUMTILES="2477" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#;
        let image = ready_image(
            "http://example.com/images/manuscript123/ImageProperties.xml",
            contents,
        );
        assert_eq!(image.title.as_deref(), Some("manuscript123"));
        let TileSource::Grid(plan) = &image.levels[5].source else {
            unreachable!()
        };
        let urls: std::collections::HashSet<_> = plan
            .tiles_row_major()
            .map(Result::unwrap)
            .map(|tile| tile.request.uri)
            .collect();
        assert!(urls.contains("http://example.com/images/manuscript123/TileGroup1/5-0-14.jpg"));
        assert!(urls.contains("http://example.com/images/manuscript123/TileGroup2/5-0-15.jpg"));
        let image = ready_image(
            "https://library.example.edu/viewer/book_of_kells/ImageProperties.xml?cache=false",
            br#"<IMAGE_PROPERTIES WIDTH="2000" HEIGHT="3000" NUMTILES="100" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#,
        );
        assert_eq!(image.title.as_deref(), Some("book_of_kells"));
        let image = ready_image(
            "http://example.com/ImageProperties.xml",
            br#"<IMAGE_PROPERTIES WIDTH="500" HEIGHT="500" NUMTILES="9" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#,
        );
        assert_eq!(image.title.as_deref(), Some("example.com"));
        let contents = br#"<IMAGE_PROPERTIES WIDTH="500" HEIGHT="500" NUMTILES="9" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#;
        let image = ready_image("http://example.com/ImageProperties.xml", contents);
        assert_eq!(
            image.warnings,
            ["Zoomify tile count mismatch: computed 5, metadata declares 9"]
        );
    }

    #[test]
    fn full_resolution_only_numtiles_uses_the_full_level() {
        let image = ready_image(
            "https://fixtures.test/zoomify-full-numtiles/ImageProperties.xml",
            br#"<IMAGE_PROPERTIES WIDTH="10240" HEIGHT="1792" NUMTILES="280" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#,
        );
        let TileSource::Grid(plan) = &image.levels.last().unwrap().source else {
            unreachable!()
        };
        assert_eq!(plan.count(), 280);
        let first = plan.tiles_row_major().next().unwrap().unwrap();
        assert_eq!(
            first.request.uri,
            "https://fixtures.test/zoomify-full-numtiles/TileGroup0/6-0-0.jpg"
        );
        let urls: Vec<_> = plan
            .tiles_row_major()
            .map(Result::unwrap)
            .map(|tile| tile.request.uri)
            .collect();
        assert!(urls.iter().all(|url| url.contains("/TileGroup0/")));
        assert!(urls.iter().any(|url| url.ends_with("/6-16-6.jpg")));
    }
}
