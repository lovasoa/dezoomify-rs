//! Pure discovery for Zoomify `ImageProperties.xml` pyramids.

use std::sync::Arc;

use image_properties::ImageProperties;

use crate::Vec2d;
use crate::core::discovery::DiscoveryEvent;
use crate::core::{
    CatalogEntry, Dezoomer, DezoomerMeta, DiscoveryDiagnostic, DiscoveryError, DiscoveryInput,
    DiscoveryStep, Grid, GridRequests, GridTile, ImageCatalog, ImageDescriptor, LevelDescriptor,
    Request, ResourceOutcome, ResourceRequest, StableId,
};

mod image_properties;

/// Zoomify metadata dezoomer.
pub struct Zoomify {
    uri: String,
    requested: bool,
}

impl Dezoomer for Zoomify {
    fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
        match event {
            DiscoveryEvent::Start if !self.uri.contains("/ImageProperties.xml") => {
                Ok(DiscoveryStep::Reject(DiscoveryDiagnostic::from(
                    "not a Zoomify ImageProperties.xml URL",
                )))
            }
            DiscoveryEvent::Start if !self.requested => {
                self.requested = true;
                Ok(DiscoveryStep::Need(ResourceRequest::new(self.uri.clone())))
            }
            DiscoveryEvent::Resource(ResourceOutcome::Response(response)) => {
                load_catalog(&self.uri, &response.bytes).map(DiscoveryStep::Complete)
            }
            DiscoveryEvent::Resource(ResourceOutcome::Failure(failure)) => {
                Err(DiscoveryError::Session(failure.message.clone()))
            }
            DiscoveryEvent::Start => Err(DiscoveryError::Session(
                "Zoomify session started twice".into(),
            )),
        }
    }
}

impl DezoomerMeta for Zoomify {
    const NAME: &'static str = "zoomify";
    const URL_HINTS: &'static [&'static str] = &["ImageProperties.xml", "TileGroup"];

    fn start(input: &DiscoveryInput) -> Self {
        Self {
            uri: input.uri.clone(),
            requested: false,
        }
    }
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
    let (level_info, warnings) = properties.levels_with_warnings();
    let levels = level_info
        .into_iter()
        .enumerate()
        .map(|(index, info)| {
            let size = info.size;
            let tile_size = info.tile_size;
            let id = StableId::new(format!("zoomify:{index}"));
            let level = ZoomifyLevel {
                base_url: Arc::clone(&base_url),
                tiles_before: info.tiles_before,
                index,
            };
            let source = Grid::new(id.clone(), size, tile_size, Vec2d::default(), level).map_err(
                |error| DiscoveryError::Session(format!("invalid Zoomify grid: {error}")),
            )?;
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

#[derive(Debug)]
struct ZoomifyLevel {
    base_url: Arc<str>,
    tiles_before: u32,
    index: usize,
}

impl GridRequests for ZoomifyLevel {
    fn request(&self, tile: GridTile) -> Request {
        let cell: Vec2d = tile.coord.into();
        let tile_group = (u64::from(self.tiles_before) + tile.row_major_ordinal) / 256;
        Request::new(format!(
            "{}/TileGroup{}/{}-{}-{}.jpg",
            self.base_url, tile_group, self.index, cell.x, cell.y
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::TileSource;

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
}
