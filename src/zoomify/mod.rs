use std::sync::Arc;

use custom_error::custom_error;
use image_properties::{ImageProperties, ZoomLevelInfo};

use crate::dezoomer::{Dezoomer, DezoomerError, DezoomerInput, DezoomerInputWithContents, TileProvider, TileReference, TilesRect, Vec2d, ZoomLevels};

mod image_properties;

#[derive(Default)]
pub struct ZoomifyDezoomer;

impl Dezoomer for ZoomifyDezoomer {
    fn name(&self) -> &'static str { "zoomify" }

    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        self.assert(data.uri.contains("/ImageProperties.xml"))?;
        let DezoomerInputWithContents { uri, contents } = data.with_contents()?;
        let levels = load_from_properties(uri, contents)?;
        Ok(levels)
    }
}

custom_error! {pub ZoomifyError
    XmlError{source: serde_xml_rs::Error} = "Unable to parse ImageProperties.xml: {source}"
}

impl From<ZoomifyError> for DezoomerError {
    fn from(err: ZoomifyError) -> Self {
        DezoomerError::Other { source: err.into() }
    }
}

fn load_from_properties(url: &str, contents: &[u8]) -> Result<ZoomLevels, ZoomifyError> {
    let image_properties_raw: ImageProperties = serde_xml_rs::from_reader(contents)?;
    let image_properties = &Arc::new(image_properties_raw);
    let base_url = &Arc::new(url.split("/ImageProperties.xml").next().unwrap().into());
    let reversed_levels: Vec<ZoomLevelInfo> = image_properties.levels().collect();
    let levels: ZoomLevels = reversed_levels.into_iter()
        .rev()
        .enumerate()
        .map(move |(level, level_info)| {
            Box::new(ZoomifyLevel {
                base_url: Arc::clone(base_url),
                level_info,
                image_properties: Arc::clone(image_properties),
                level,
            }) as Box<dyn TileProvider + Sync>
        }).collect();
    Ok(levels)
}

struct ZoomifyLevel {
    base_url: Arc<String>,
    level_info: ZoomLevelInfo,
    image_properties: Arc<ImageProperties>,
    level: usize,
}

impl TilesRect for ZoomifyLevel {
    fn size(&self) -> Vec2d { self.level_info.size }

    fn tile_size(&self) -> Vec2d { self.level_info.tile_size }

    fn tile_url(&self, pos: Vec2d) -> String {
        format!("{base}/TileGroup{group}/{z}-{x}-{y}.jpg",
                base = self.base_url,
                group = self.level_info.tile_group(pos),
                x = pos.x,
                y = pos.y,
                z = self.level
        )
    }
}

impl std::fmt::Debug for ZoomifyLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Zoomify Image")
    }
}