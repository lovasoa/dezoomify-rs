//! Pure Google Arts & Culture two-stage discovery.

use crate::Vec2d;
use crate::core::discovery::DiscoveryEvent;
use crate::core::tile_plan::RectangularSource;
use crate::core::{
    CatalogEntry, DiscoveryDiagnostic, DiscoveryError, DiscoveryInput, DiscoveryProgram,
    DiscoverySession, DiscoveryStep, ImageCatalog, ImageDescriptor, KnownTilePlan, LevelDescriptor,
    LevelPlan, ProcessingRecipe, Provenance, Request, ResourceOutcome, ResourcePurpose,
    ResourceRequest, StableId,
};
use std::sync::Arc;
use tile_info::{PageInfo, TileInfo};
mod tile_info;
mod url;

#[derive(Default)]
pub struct GAPDezoomer;
impl DiscoveryProgram for GAPDezoomer {
    fn start(&self, input: &DiscoveryInput) -> Box<dyn DiscoverySession> {
        Box::new(GapSession {
            uri: input.uri.clone(),
            page: None,
            requested: false,
        })
    }
}
struct GapSession {
    uri: String,
    page: Option<Arc<PageInfo>>,
    requested: bool,
}
impl DiscoverySession for GapSession {
    fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
        match event {
            DiscoveryEvent::Start
                if !self.uri.contains("artsandculture.google.com") && !self.uri.ends_with("=g") =>
            {
                Ok(DiscoveryStep::Reject(DiscoveryDiagnostic::from(
                    "not a Google Arts & Culture URL",
                )))
            }
            DiscoveryEvent::Start if !self.requested => {
                self.requested = true;
                Ok(DiscoveryStep::Need(ResourceRequest::new(
                    self.uri.clone(),
                    ResourcePurpose::InitialMetadata,
                )))
            }
            DiscoveryEvent::Resource(ResourceOutcome::Response(response))
                if self.page.is_none() =>
            {
                let source = std::str::from_utf8(&response.bytes)
                    .map_err(|error| DiscoveryError::Session(error.to_string()))?;
                let page: PageInfo = source
                    .parse::<PageInfo>()
                    .map_err(|error| DiscoveryError::Session(error.to_string()))?;
                let next = page.tile_info_url();
                self.page = Some(Arc::new(page));
                Ok(DiscoveryStep::Need(ResourceRequest::new(
                    next,
                    ResourcePurpose::TileInformation,
                )))
            }
            DiscoveryEvent::Resource(ResourceOutcome::Response(response)) => {
                catalog(self.page.as_ref().expect("page set"), &response.bytes)
                    .map(DiscoveryStep::Complete)
            }
            DiscoveryEvent::Resource(ResourceOutcome::Failure(failure)) => {
                Err(DiscoveryError::Session(failure.message.clone()))
            }
            DiscoveryEvent::Start => {
                Err(DiscoveryError::Session("GAP session started twice".into()))
            }
        }
    }
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
            let id = StableId::new(format!("gap:{z}"));
            let source = GapLevel {
                size,
                tile_size: Vec2d {
                    x: tile_width,
                    y: tile_height,
                },
                z,
                page: Arc::clone(page),
                id: id.clone(),
            };
            LevelDescriptor {
                id,
                title: None,
                size: Some(size),
                tile_size: Some(source.tile_size),
                scale_factor: None,
                has_overlapping_tiles: false,
                plan: LevelPlan::Known(KnownTilePlan::rectangular(source)),
                provenance: Provenance::default(),
                warnings: Vec::new(),
            }
        })
        .collect();
    levels.sort_by_key(|level| level.size.map_or(0, Vec2d::area));
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("gap:image"),
        title: Some(page.name.clone()),
        format: StableId::new("google_arts_and_culture"),
        levels,
        provenance: Provenance::default(),
        warnings: Vec::new(),
    })]))
}
#[derive(Debug)]
struct GapLevel {
    size: Vec2d,
    tile_size: Vec2d,
    z: usize,
    page: Arc<PageInfo>,
    id: StableId,
}
impl RectangularSource for GapLevel {
    fn level_id(&self) -> StableId {
        self.id.clone()
    }
    fn image_size(&self) -> Vec2d {
        self.size
    }
    fn tile_size(&self) -> Vec2d {
        self.tile_size
    }
    fn request(&self, cell: Vec2d) -> Request {
        Request::new(url::compute_url(&self.page, cell.x, cell.y, self.z))
    }
    fn processing(&self) -> ProcessingRecipe {
        ProcessingRecipe::Named(StableId::new("google-arts-decrypt"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{RequestId, ResourceResponse};

    fn response(bytes: &[u8]) -> ResourceOutcome {
        ResourceOutcome::Response(ResourceResponse {
            id: RequestId(0),
            bytes: bytes.to_vec(),
            content_type: None,
        })
    }

    fn fixture_catalog() -> ImageCatalog {
        let input = DiscoveryInput::from("https://artsandculture.google.com/asset/test");
        let mut session = GAPDezoomer.start(&input);
        session.advance(DiscoveryEvent::Start).unwrap();
        let page = response(include_bytes!(
            "../../testdata/google_arts_and_culture/page_source.html"
        ));
        session.advance(DiscoveryEvent::Resource(&page)).unwrap();
        let tile_info = response(include_bytes!(
            "../../testdata/google_arts_and_culture/tile_info.xml"
        ));
        let DiscoveryStep::Complete(catalog) = session
            .advance(DiscoveryEvent::Resource(&tile_info))
            .unwrap()
        else {
            panic!("fixture tile information must complete discovery");
        };
        catalog
    }

    #[test]
    fn discovers_fixture_as_five_replayable_levels() {
        let input = DiscoveryInput::from("https://artsandculture.google.com/asset/test");
        let mut session = GAPDezoomer.start(&input);
        let DiscoveryStep::Need(page) = session.advance(DiscoveryEvent::Start).unwrap() else {
            panic!("Google Arts discovery must request the page");
        };
        assert_eq!(page.request.uri, input.uri);

        let page = response(include_bytes!(
            "../../testdata/google_arts_and_culture/page_source.html"
        ));
        let DiscoveryStep::Need(tile_info) =
            session.advance(DiscoveryEvent::Resource(&page)).unwrap()
        else {
            panic!("the page must lead to tile information");
        };
        assert!(tile_info.request.uri.ends_with("=g"));

        let tile_info = response(include_bytes!(
            "../../testdata/google_arts_and_culture/tile_info.xml"
        ));
        let DiscoveryStep::Complete(catalog) = session
            .advance(DiscoveryEvent::Resource(&tile_info))
            .unwrap()
        else {
            panic!("tile information must complete discovery");
        };
        let [CatalogEntry::Ready(image)] = catalog.entries() else {
            panic!("Google Arts produces one ready image");
        };
        assert_eq!(image.levels.len(), 5);
        assert!(
            image
                .levels
                .windows(2)
                .all(|levels| { levels[0].size.unwrap().area() <= levels[1].size.unwrap().area() })
        );
        let LevelPlan::Known(plan) = &image.levels[0].plan else {
            panic!("Google Arts geometry is known");
        };
        let tile = plan.cursor().take_ready(1).unwrap().unwrap().pop().unwrap();
        assert_eq!(
            tile.processing,
            ProcessingRecipe::Named(StableId::new("google-arts-decrypt"))
        );
    }

    #[test]
    fn rejects_non_google_urls_without_requesting_data() {
        let input = DiscoveryInput::from("https://example.com/test");
        let mut session = GAPDezoomer.start(&input);
        assert!(matches!(
            session.advance(DiscoveryEvent::Start).unwrap(),
            DiscoveryStep::Reject(_)
        ));
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
        assert_eq!(level.size, Some(Vec2d { x: 5436, y: 4080 }));
        assert_eq!(level.tile_size, Some(Vec2d { x: 512, y: 512 }));
        let LevelPlan::Known(plan) = &level.plan else {
            panic!("Google Arts geometry is known");
        };
        assert_eq!(plan.len(), 88);
        let first = plan.tile(0).unwrap().unwrap();
        assert_eq!(first.destination, Vec2d::default());
        assert_eq!(first.expected_size, Some(Vec2d { x: 512, y: 512 }));
        assert!(first.request.uri.contains("=x0-y0-z4-t"));
        assert_eq!(
            first.processing,
            ProcessingRecipe::Named(StableId::new("google-arts-decrypt"))
        );
        let last = plan.tile(87).unwrap().unwrap();
        assert_eq!(last.destination, Vec2d { x: 5120, y: 3584 });
        assert_eq!(last.expected_size, Some(Vec2d { x: 316, y: 496 }));
        assert!(last.request.uri.contains("=x10-y7-z4-t"));
    }

    #[test]
    fn invalid_tile_information_is_reported_as_a_session_error() {
        let input = DiscoveryInput::from("https://artsandculture.google.com/asset/test");
        let mut session = GAPDezoomer.start(&input);
        session.advance(DiscoveryEvent::Start).unwrap();
        let page = response(include_bytes!(
            "../../testdata/google_arts_and_culture/page_source.html"
        ));
        session.advance(DiscoveryEvent::Resource(&page)).unwrap();
        let invalid = response(b"<invalid>not a tile info</invalid>");
        let Err(DiscoveryError::Session(message)) =
            session.advance(DiscoveryEvent::Resource(&invalid))
        else {
            panic!("invalid tile XML must be rejected by the pure session");
        };
        assert!(message.contains("invalid Google Arts tile XML"));
    }
}
