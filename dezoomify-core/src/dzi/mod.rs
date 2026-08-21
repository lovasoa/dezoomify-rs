//! Pure discovery for Deep Zoom Image descriptors.

use std::sync::Arc;

use dzi_file::DziFile;
use regex::Regex;

use crate::Vec2d;
use crate::core::discovery::DiscoveryEvent;
use crate::core::tile_plan::RectangularSource;
use crate::core::{
    CatalogEntry, DiscoveryError, DiscoveryInput, DiscoveryProgram, DiscoverySession,
    DiscoveryStep, ImageCatalog, ImageDescriptor, KnownTilePlan, LevelDescriptor, LevelPlan,
    ProcessingRecipe, Request, ResourceOutcome, ResourceRequest,
    StableId,
};
use crate::json_utils::all_json;

mod dzi_file;

/// Deep Zoom Image discovery program.
#[derive(Default)]
pub struct DziDezoomer;

impl DiscoveryProgram for DziDezoomer {
    fn start(&self, input: &DiscoveryInput) -> Box<dyn DiscoverySession> {
        Box::new(DziSession {
            input: input.uri.clone(),
            requested: false,
            metadata_uri: None,
        })
    }
}

struct DziSession {
    input: String,
    requested: bool,
    metadata_uri: Option<String>,
}

impl DiscoverySession for DziSession {
    fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
        match event {
            DiscoveryEvent::Start if !self.requested => {
                self.requested = true;
                let tile = Regex::new("_files/\\d+/\\d+_\\d+\\.(jpe?g|png)$")
                    .expect("constant DZI tile pattern");
                let uri = tile.find(&self.input).map_or_else(
                    || self.input.clone(),
                    |matched| format!("{}.dzi", &self.input[..matched.start()]),
                );
                self.metadata_uri = Some(uri.clone());
                Ok(DiscoveryStep::Need(ResourceRequest::new(
                    uri,
                )))
            }
            DiscoveryEvent::Resource(ResourceOutcome::Response(response)) => load_catalog(
                self.metadata_uri.as_deref().unwrap_or(&self.input),
                &response.bytes,
            )
            .map(DiscoveryStep::Complete),
            DiscoveryEvent::Resource(ResourceOutcome::Failure(failure)) => {
                Err(DiscoveryError::Session(failure.message.clone()))
            }
            DiscoveryEvent::Start => {
                Err(DiscoveryError::Session("DZI session started twice".into()))
            }
        }
    }
}

fn load_catalog(url: &str, contents: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let xml_result = serde_xml_rs::from_reader::<'_, DziFile, _>(contents);
    let xml_err = xml_result.as_ref().err().map(ToString::to_string);
    let parsed = xml_result
        .ok()
        .into_iter()
        .chain(all_json::<DziFile>(contents))
        .collect::<Vec<_>>();
    if parsed.is_empty() {
        let detail = xml_err
            .map(|e| format!(": {e}"))
            .unwrap_or_default();
        return Err(DiscoveryError::Session(format!(
            "unable to parse DZI metadata{detail}"
        )));
    }
    let mut entries = Vec::new();
    for (image_index, image) in parsed.into_iter().enumerate() {
        if image.tile_size == 0 {
            return Err(DiscoveryError::Session("invalid DZI zero tile size".into()));
        }
        let base_url: Arc<str> = image.base_url(url).into();
        let image_size = image.get_size();
        let tile_size = image.get_tile_size();
        let max_level = image.max_level();
        let mut levels: Vec<_> = std::iter::successors(Some(image_size), |size| {
            (size.x > 1 || size.y > 1).then(|| size.ceil_div(Vec2d::square(2)))
        })
        .zip((0..=max_level).rev())
        .enumerate()
        .map(|(ordinal, (size, zoom))| {
            let id = StableId::new(format!("dzi:{image_index}:{ordinal}"));
            let level = DziLevel {
                base_url: Arc::clone(&base_url),
                size,
                tile_size,
                format: image.format.clone(),
                overlap: image.overlap,
                zoom,
                id: id.clone(),
            };
            LevelDescriptor {
                id,
                title: Some(format!("DZI level {ordinal} ({}×{} pixels)", size.x, size.y)),
                size: Some(size),
                tile_size: Some(tile_size),
                has_overlapping_tiles: image.overlap > 0,
                plan: LevelPlan::Known(KnownTilePlan::rectangular(level)),
                ..Default::default()
            }
        })
        .collect();
        levels.reverse();
        let title = base_url
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .map(|s| s.trim_end_matches("_files").to_owned());
        entries.push(CatalogEntry::Ready(ImageDescriptor {
            id: StableId::new(format!("dzi:{image_index}")),
            title,
            format: StableId::new("deepzoom"),
            levels,
            ..Default::default()
        }));
    }
    Ok(ImageCatalog::new(entries))
}

#[derive(Debug)]
struct DziLevel {
    base_url: Arc<str>,
    size: Vec2d,
    tile_size: Vec2d,
    format: String,
    overlap: u32,
    zoom: u32,
    id: StableId,
}

impl RectangularSource for DziLevel {
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
        Request::new(format!(
            "{}/{}/{}_{}.{}",
            self.base_url, self.zoom, cell.x, cell.y, self.format
        ))
    }
    fn overlap(&self) -> Vec2d {
        Vec2d::square(self.overlap)
    }
    fn processing(&self) -> ProcessingRecipe {
        ProcessingRecipe::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_image(catalog: ImageCatalog) -> ImageDescriptor {
        match catalog.into_entries().pop().unwrap() {
            CatalogEntry::Ready(image) => image,
            CatalogEntry::Deferred(_) => panic!("DZI is resolved"),
        }
    }

    #[test]
    fn panorama_preserves_urls_overlap_and_normalized_level_order() {
        let contents = br#"<Image TileSize="256" Overlap="2" Format="jpg"><Size Width="600" Height="300"/></Image>"#;
        let catalog = load_catalog("http://x.fr/y/test.dzi", contents).unwrap();
        let CatalogEntry::Ready(image) = &catalog.entries()[0] else {
            panic!("DZI is immediately ready")
        };
        assert_eq!(image.title.as_deref(), Some("test"));
        let levels = ready_image(catalog).levels;
        assert_eq!(levels.len(), 11);
        assert!(
            levels
                .windows(2)
                .all(|pair| pair[0].size.unwrap().area() <= pair[1].size.unwrap().area())
        );
        let plan = match &levels[9].plan {
            LevelPlan::Known(plan) => plan,
            LevelPlan::Adaptive(_) => panic!("DZI is known"),
        };
        let urls: Vec<_> = plan
            .cursor()
            .take_ready(10)
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|tile| tile.request.uri)
            .collect();
        assert_eq!(
            urls,
            vec![
                "http://x.fr/y/test_files/9/0_0.jpg",
                "http://x.fr/y/test_files/9/1_0.jpg"
            ]
        );
    }

    #[test]
    fn parses_xml_with_bom_and_openseadragon_configuration() {
        let bom = "\u{feff}<Image TileSize=\"256\" Overlap=\"0\" Format=\"jpg\"><Size Width=\"6261\" Height=\"6047\"/></Image>";
        let catalog = load_catalog("http://test.com/test.xml", bom.as_bytes()).unwrap();
        assert_eq!(catalog.len(), 1);
        let image = ready_image(catalog);
        assert_eq!(
            image.levels.last().unwrap().size,
            Some(Vec2d { x: 6261, y: 6047 })
        );
        let script = r#"OpenSeadragon({tileSources:{Image:{Url:"/example-images/highsmith/highsmith_files/",Format:"jpg",Overlap:"2",TileSize:"256",Size:{Height:"9221",Width:"7026"}}}});"#;
        let levels =
            ready_image(load_catalog("http://test.com/x/test.xml", script.as_bytes()).unwrap())
                .levels;
        let large = levels.last().unwrap();
        assert_eq!(large.size, Some(Vec2d { x: 7026, y: 9221 }));
        let plan = match &large.plan {
            LevelPlan::Known(plan) => plan,
            LevelPlan::Adaptive(_) => unreachable!(),
        };
        assert_eq!(
            plan.cursor().take_ready(1).unwrap().unwrap()[0].request.uri,
            "http://test.com/example-images/highsmith/highsmith_files/14/0_0.jpg"
        );
    }
}
