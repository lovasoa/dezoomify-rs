use std::sync::Arc;

use custom_error::custom_error;
use image_properties::{ImageProperties, ZoomLevelInfo};

use crate::dezoomer::*;

mod image_properties;

/// Dezoomer for the zoomify image format.
/// See: <http://zoomify.com/>
#[derive(Default)]
pub struct ZoomifyDezoomer;

impl Dezoomer for ZoomifyDezoomer {
    fn name(&self) -> &'static str {
        "zoomify"
    }

    fn images(&mut self, data: &DezoomerInput) -> Result<Images, DezoomerError> {
        self.assert(data.uri.contains("/ImageProperties.xml"))?;
        let DezoomerInputWithContents { uri, contents } = data.with_contents()?;
        let levels = load_from_properties(uri, contents)?;
        Ok(levels.into())
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
    let base_url_string = url
        .split("/ImageProperties.xml")
        .next()
        .unwrap()
        .to_string();
    let base_url = &Arc::from(base_url_string);
    let levels: Vec<ZoomLevelInfo> = image_properties.levels();
    let levels: ZoomLevels = levels
        .into_iter()
        .enumerate()
        .map(move |(level, level_info)| ZoomifyLevel {
            base_url: Arc::clone(base_url),
            level_info,
            level,
        })
        .into_zoom_levels();
    Ok(levels)
}

struct ZoomifyLevel {
    base_url: Arc<str>,
    level_info: ZoomLevelInfo,
    level: usize,
}

impl TilesRect for ZoomifyLevel {
    fn size(&self) -> Vec2d {
        self.level_info.size
    }

    fn tile_size(&self) -> Vec2d {
        self.level_info.tile_size
    }

    fn tile_url(&self, pos: Vec2d) -> String {
        format!(
            "{base}/TileGroup{group}/{z}-{x}-{y}.jpg",
            base = self.base_url,
            group = self.level_info.tile_group(pos),
            x = pos.x,
            y = pos.y,
            z = self.level
        )
    }

    fn title(&self) -> Option<String> {
        // Extract a meaningful title from the base URL
        // For Zoomify, URLs typically end with the image name before "/ImageProperties.xml"
        // e.g., "https://example.com/images/myimage/ImageProperties.xml" -> "myimage"
        let url_with_trailing_slash = format!("{}/", self.base_url);
        let path = url_with_trailing_slash
            .split('/')
            .rfind(|s| !s.is_empty())
            .unwrap_or("zoomify_image");

        Some(path.to_string())
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
    let mut props = load_from_properties(url, contents).unwrap();
    assert_eq!(props.len(), 11);
    let level = &mut props[3];
    let tiles: Vec<String> = level.next_tiles(None).into_iter().map(|t| t.url).collect();
    assert_eq!(
        tiles,
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
fn test_tilegroups() {
    use std::collections::HashSet;
    let url = "http://x.fr/y/ImageProperties.xml?t";
    let contents = br#"<IMAGE_PROPERTIES WIDTH="12000" HEIGHT="9788"
                                NUMTILES="2477" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#;
    let mut props = load_from_properties(url, contents).unwrap();
    let level = &mut props[5];
    let tiles: HashSet<String> = level.next_tiles(None).into_iter().map(|t| t.url).collect();
    assert!(tiles.contains("http://x.fr/y/TileGroup1/5-0-14.jpg"));
    assert!(tiles.contains("http://x.fr/y/TileGroup2/5-0-15.jpg"));
}

#[test]
fn test_title_extraction() {
    let url = "http://example.com/images/manuscript123/ImageProperties.xml";
    let contents = br#"<IMAGE_PROPERTIES WIDTH="1000" HEIGHT="1000"
                                NUMTILES="25" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#;
    let mut props = load_from_properties(url, contents).unwrap();
    let level = &mut props[0];

    // Test that the title is extracted from the URL path
    assert_eq!(level.title(), Some("manuscript123".to_string()));
}

#[test]
fn test_title_extraction_with_query_params() {
    let url = "https://library.example.edu/viewer/book_of_kells/ImageProperties.xml?cache=false";
    let contents = br#"<IMAGE_PROPERTIES WIDTH="2000" HEIGHT="3000"
                                NUMTILES="100" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#;
    let mut props = load_from_properties(url, contents).unwrap();
    let level = &mut props[0];

    // Test that the title ignores query parameters
    assert_eq!(level.title(), Some("book_of_kells".to_string()));
}

#[test]
fn test_title_extraction_simple_path() {
    let url = "http://example.com/ImageProperties.xml";
    let contents = br#"<IMAGE_PROPERTIES WIDTH="500" HEIGHT="500"
                                NUMTILES="9" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#;
    let mut props = load_from_properties(url, contents).unwrap();
    let level = &mut props[0];

    // Test fallback when no meaningful path is found
    assert_eq!(level.title(), Some("example.com".to_string()));
}
