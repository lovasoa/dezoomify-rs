//! Pure discovery for `IIPImage` metadata and tile pyramids.

use regex::Regex;
use std::str::FromStr;
use std::sync::Arc;

use crate::Vec2d;
use crate::core::discovery::DiscoveryEvent;
use crate::core::tile_plan::RectangularSource;
use crate::core::{
    CatalogEntry, DiscoveryDiagnostic, DiscoveryError, DiscoveryInput, DiscoveryProgram,
    DiscoverySession, DiscoveryStep, ImageCatalog, ImageDescriptor, KnownTilePlan, LevelDescriptor,
    LevelPlan, ProcessingRecipe, Request, ResourceOutcome,
    ResourceRequest, StableId,
};

/// `IIPImage` discovery program.
#[derive(Default)]
pub struct IIPImage;
const META: &str = "&OBJ=Max-size&OBJ=Tile-size&OBJ=Resolution-number";

impl DiscoveryProgram for IIPImage {
    fn start(&self, input: &DiscoveryInput) -> Box<dyn DiscoverySession> {
        Box::new(IipSession {
            input: input.uri.clone(),
            metadata: None,
        })
    }
}
struct IipSession {
    input: String,
    metadata: Option<String>,
}
impl DiscoverySession for IipSession {
    fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
        match event {
            DiscoveryEvent::Start if self.metadata.is_none() => {
                let meta = if self.input.ends_with(META) {
                    self.input.clone()
                } else {
                    if !Regex::new("(?i)\\?FIF")
                        .expect("constant IIP pattern")
                        .is_match(&self.input)
                    {
                        return Ok(DiscoveryStep::Reject(DiscoveryDiagnostic::from(
                            "not an IIPImage URL",
                        )));
                    }
                    format!(
                        "{}{}",
                        self.input
                            .chars()
                            .take_while(|character| *character != '&')
                            .collect::<String>(),
                        META
                    )
                };
                self.metadata = Some(meta.clone());
                Ok(DiscoveryStep::Need(ResourceRequest::new(
                    meta,
                )))
            }
            DiscoveryEvent::Resource(ResourceOutcome::Response(response)) => catalog(
                self.metadata.as_deref().unwrap_or(&self.input),
                &response.bytes,
            )
            .map(DiscoveryStep::Complete),
            DiscoveryEvent::Resource(ResourceOutcome::Failure(failure)) => {
                Err(DiscoveryError::Session(failure.message.clone()))
            }
            DiscoveryEvent::Start => {
                Err(DiscoveryError::Session("IIP session started twice".into()))
            }
        }
    }
}

fn catalog(uri: &str, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let metadata = Arc::new(Metadata::try_from(bytes)?);
    let base: Arc<str> = uri.trim_end_matches(META).into();
    let mut levels: Vec<_> = (0..metadata.levels)
        .map(|index| {
            let reverse = metadata.levels - index - 1;
            let size = metadata.size / 2_u32.pow(reverse);
            let id = StableId::new(format!("iip:{index}"));
            let source = IipLevel {
                metadata: Arc::clone(&metadata),
                base: Arc::clone(&base),
                index,
                id: id.clone(),
            };
            LevelDescriptor {
                id,
                title: Some(format!("IIP level {index} ({}×{} pixels)", size.x, size.y)),
                size: Some(size),
                tile_size: Some(metadata.tile_size),
                plan: LevelPlan::Known(KnownTilePlan::rectangular(source)),
                ..Default::default()
            }
        })
        .collect();
    levels.sort_by_key(|level| level.size.map_or(0, Vec2d::area));
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("iip:image"),
        format: StableId::new("iipimage"),
        levels,
        ..Default::default()
    })]))
}

#[derive(Clone, Debug)]
struct IipLevel {
    metadata: Arc<Metadata>,
    base: Arc<str>,
    index: u32,
    id: StableId,
}
impl RectangularSource for IipLevel {
    fn level_id(&self) -> StableId {
        self.id.clone()
    }
    fn image_size(&self) -> Vec2d {
        self.metadata.size / 2_u32.pow(self.metadata.levels - self.index - 1)
    }
    fn tile_size(&self) -> Vec2d {
        self.metadata.tile_size
    }
    fn request(&self, cell: Vec2d) -> Request {
        let width = self.image_size().ceil_div(self.tile_size()).x;
        Request::new(format!(
            "{}&JTL={},{}",
            self.base,
            self.index,
            cell.y * width + cell.x
        ))
    }
    fn processing(&self) -> ProcessingRecipe {
        ProcessingRecipe::None
    }
}

#[derive(Clone, Debug)]
struct Metadata {
    size: Vec2d,
    tile_size: Vec2d,
    levels: u32,
}
impl FromStr for Metadata {
    type Err = DiscoveryError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let numbers = |name: &str, minimum: usize| {
            value
                .lines()
                .filter_map(|line| {
                    let (key, raw) = line.split_once(':')?;
                    let values: Vec<_> = raw
                        .split_ascii_whitespace()
                        .filter_map(|number| number.parse::<u32>().ok())
                        .collect();
                    (key.trim().eq_ignore_ascii_case(name) && values.len() >= minimum)
                        .then_some(values)
                })
                .next_back()
        };
        let size = numbers("max-size", 2).map(|values| Vec2d {
            x: values[0],
            y: values[1],
        });
        let tile_size = numbers("tile-size", 2).map(|values| Vec2d {
            x: values[0],
            y: values[1],
        });
        let levels = numbers("resolution-number", 1).map(|values| values[0]);
        Ok(Self {
            size: size
                .ok_or_else(|| DiscoveryError::Session("IIP metadata lacks Max-size".into()))?,
            tile_size: tile_size
                .ok_or_else(|| DiscoveryError::Session("IIP metadata lacks Tile-size".into()))?,
            levels: levels.ok_or_else(|| {
                DiscoveryError::Session("IIP metadata lacks Resolution-number".into())
            })?,
        })
    }
}
impl TryFrom<&[u8]> for Metadata {
    type Error = DiscoveryError;
    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        std::str::from_utf8(bytes)
            .map_err(|error| DiscoveryError::Session(error.to_string()))?
            .parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercase_fif_urls_request_canonical_metadata() {
        let uri = "https://publications-images.artic.edu/fcgi-bin/iipsrv.fcgi?fif=osci/Renoir_11/Color_Corrected/G39094sm2.ptif&jtl=4,11";
        let mut session = IIPImage.start(&DiscoveryInput::from(uri));
        let step = session.advance(DiscoveryEvent::Start).unwrap();
        let DiscoveryStep::Need(need) = step else {
            panic!("IIP URL did not request metadata")
        };
        assert_eq!(
            need.request.uri,
            "https://publications-images.artic.edu/fcgi-bin/iipsrv.fcgi?fif=osci/Renoir_11/Color_Corrected/G39094sm2.ptif&OBJ=Max-size&OBJ=Tile-size&OBJ=Resolution-number"
        );
    }

    #[test]
    fn parses_metadata_levels_and_iip_tile_geometry() {
        let metadata = b"Max-size:512 512\nTile-size:256 256\nResolution-number:2";
        let catalog = catalog(
            "http://test.com/&OBJ=Max-size&OBJ=Tile-size&OBJ=Resolution-number",
            metadata,
        )
        .unwrap();
        let CatalogEntry::Ready(image) = catalog.into_entries().pop().unwrap() else {
            panic!("IIP metadata did not produce an image")
        };
        assert_eq!(image.levels.len(), 2);
        assert_eq!(image.levels[0].size, Some(Vec2d { x: 256, y: 256 }));
        assert_eq!(image.levels[1].size, Some(Vec2d { x: 512, y: 512 }));
        let LevelPlan::Known(low_plan) = &image.levels[0].plan else {
            panic!("IIP levels must have known plans")
        };
        assert_eq!(low_plan.len(), 1);
        assert_eq!(
            low_plan.tile(0).unwrap().unwrap().request.uri,
            "http://test.com/&JTL=0,0"
        );
        let LevelPlan::Known(plan) = &image.levels[1].plan else {
            panic!("IIP levels must have known plans")
        };
        assert_eq!(plan.len(), 4);
        assert_eq!(
            plan.tile(0).unwrap().unwrap().request.uri,
            "http://test.com/&JTL=1,0"
        );
        assert_eq!(
            plan.tile(2).unwrap().unwrap().request.uri,
            "http://test.com/&JTL=1,2"
        );
    }

    #[test]
    fn metadata_parsing_reports_missing_fields_and_invalid_utf8() {
        let error = "Max-size:512 512"
            .parse::<Metadata>()
            .unwrap_err()
            .to_string();
        assert!(error.contains("Tile-size"));
        let error = Metadata::try_from(&[0xff][..]).unwrap_err().to_string();
        assert!(error.contains("UTF") || error.contains("utf"));
        let parsed: Metadata = "Max-size:23235 23968\nTile-size:256 256\nResolution-number:9"
            .parse()
            .unwrap();
        assert_eq!(parsed.size, Vec2d { x: 23235, y: 23968 });
        assert_eq!(parsed.tile_size, Vec2d { x: 256, y: 256 });
        assert_eq!(parsed.levels, 9);
    }
}
