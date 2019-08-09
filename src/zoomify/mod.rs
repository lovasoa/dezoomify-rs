use std::sync::Arc;

use custom_error::custom_error;
use image_properties::{ImageProperties, ZoomLevelInfo};

use crate::dezoomer::*;

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
    let image_properties: ImageProperties = serde_xml_rs::from_reader(contents)?;
    let base_url = &Arc::new(url.split("/ImageProperties.xml").next().unwrap().into());
    let reversed_levels: Vec<ZoomLevelInfo> = image_properties.levels().collect();
    let levels: ZoomLevels = reversed_levels.into_iter()
        .rev()
        .enumerate()
        .map(move |(level, level_info)| {
            ZoomifyLevel {
                base_url: Arc::clone(base_url),
                level_info,
                level,
            }
        }).into_zoom_levels();
    Ok(levels)
}

struct ZoomifyLevel {
    base_url: Arc<String>,
    level_info: ZoomLevelInfo,
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

#[test]
fn test_panorama() {
    let url = "http://x.fr/y/ImageProperties.xml?t";
    let contents = br#"
        <IMAGE_PROPERTIES
            WIDTH="174550" HEIGHT="16991" NUMTILES="61284"
            NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#;
    let props = load_from_properties(url, contents).unwrap();
    assert_eq!(props.len(), 11);
    let level = &props[3];
    let tiles: Vec<String> = level.tiles().into_iter().map(|t| t.unwrap().url).collect();
    assert_eq!(tiles, vec![
        "http://x.fr/y/TileGroup0/3-0-0.jpg",
        "http://x.fr/y/TileGroup0/3-1-0.jpg",
        "http://x.fr/y/TileGroup0/3-2-0.jpg",
        "http://x.fr/y/TileGroup0/3-3-0.jpg",
        "http://x.fr/y/TileGroup0/3-4-0.jpg",
        "http://x.fr/y/TileGroup0/3-5-0.jpg"
    ]);
}

#[test]
fn test_tilegroups() {
    let url = "http://x.fr/y/ImageProperties.xml?t";
    let contents = br#"<IMAGE_PROPERTIES WIDTH="12000" HEIGHT="9788"
                                NUMTILES="2477" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#;
    let props = load_from_properties(url, contents).unwrap();
    let level = &props[5];
    let tiles: Vec<String> = level.tiles().into_iter().map(|t| t.unwrap().url).collect();
    assert_eq!(tiles[14], "http://x.fr/y/TileGroup1/5-0-14.jpg");
    assert_eq!(tiles[15], "http://x.fr/y/TileGroup2/5-0-15.jpg");
}
