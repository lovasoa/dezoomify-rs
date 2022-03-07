use std::sync::Arc;

use custom_error::custom_error;
use log::debug;

use dzi_file::DziFile;

use crate::dezoomer::*;
use crate::json_utils::all_json;
use crate::network::remove_bom;
use regex::Regex;
mod dzi_file;

/// A dezoomer for Deep Zoom Images
/// See https://docs.microsoft.com/en-us/previous-versions/windows/silverlight/dotnet-windows-silverlight/cc645043%28v%3dvs.95%29
#[derive(Default)]
pub struct DziDezoomer;

impl Dezoomer for DziDezoomer {
    fn name(&self) -> &'static str {
        "deepzoom"
    }

    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        let tile_re = Regex::new("_files/\\d+/\\d+_\\d+\\.(jpe?g|png)$").unwrap();
        if let Some(m) = tile_re.find(&data.uri) {
            let meta_uri = data.uri[..m.start()].to_string() + ".dzi";
            debug!("'{}' looks like a dzi image tile URL. Trying to fetch the DZI file at '{}'.", data.uri, meta_uri);
            Err(DezoomerError::NeedsData { uri: meta_uri })
        } else {
            let DezoomerInputWithContents { uri, contents } = data.with_contents()?;
            let levels = load_from_properties(uri, contents)?;
            Ok(levels)
        }
    }
}

custom_error! {pub DziError
    XmlError{source: serde_xml_rs::Error} = "Unable to parse the dzi file: {source}",
    NoSize = "Expected a size in the DZI file",
    InvalidTileSize = "Invalid tile size. The tile size cannot be zero.",
}

impl From<DziError> for DezoomerError {
    fn from(err: DziError) -> Self {
        DezoomerError::Other { source: err.into() }
    }
}

fn load_from_properties(url: &str, contents: &[u8]) -> Result<ZoomLevels, DziError> {

    // Workaround for https://github.com/netvl/xml-rs/issues/155
    // which the original author seems unwilling to fix
    serde_xml_rs::from_reader::<_, DziFile>(remove_bom(contents))
        .map_err(DziError::from)
        .and_then(|dzi| load_from_dzi(url, dzi))
        .or_else(|e| {
            let levels: Vec<ZoomLevel> = all_json::<DziFile>(contents)
                .flat_map(|dzi| load_from_dzi(url, dzi))
                .flatten()
                .collect();
            if levels.is_empty() { Err(e) } else { Ok(levels) }
        })
}

fn load_from_dzi(url: &str, image_properties: DziFile) -> Result<ZoomLevels, DziError> {
    debug!("Found dzi meta-information: {:?}", image_properties);

    if image_properties.tile_size == 0 {
        return Err(DziError::InvalidTileSize);
    }

    let base_url = &Arc::from(image_properties.base_url(url));

    let size = image_properties.get_size()?;
    let max_level = image_properties.max_level();
    let levels = std::iter::successors(Some(size), |&size| {
        if size.x > 1 || size.y > 1 {
            Some(size.ceil_div(Vec2d::square(2)))
        } else {
            None
        }
    })
    .enumerate()
    .map(|(level_num, size)| DziLevel {
        base_url: Arc::clone(base_url),
        size,
        tile_size: image_properties.get_tile_size(),
        format: image_properties.format.clone(),
        overlap: image_properties.overlap,
        level: max_level - level_num as u32,
    })
    .into_zoom_levels();
    Ok(levels)
}

struct DziLevel {
    base_url: Arc<str>,
    size: Vec2d,
    tile_size: Vec2d,
    format: String,
    overlap: u32,
    level: u32,
}

impl TilesRect for DziLevel {
    fn size(&self) -> Vec2d {
        self.size
    }

    fn tile_size(&self) -> Vec2d {
        self.tile_size
    }

    fn tile_url(&self, pos: Vec2d) -> String {
        format!(
            "{base}/{level}/{x}_{y}.{format}",
            base = self.base_url,
            level = self.level,
            x = pos.x,
            y = pos.y,
            format = self.format
        )
    }

    fn tile_ref(&self, pos: Vec2d) -> TileReference {
        let delta = Vec2d {
            x: if pos.x == 0 { 0 } else { self.overlap },
            y: if pos.y == 0 { 0 } else { self.overlap },
        };
        TileReference {
            url: self.tile_url(pos),
            position: self.tile_size() * pos - delta,
        }
    }

    fn title(&self) -> Option<String> {
        let (_, suffix) = self.base_url.rsplit_once( '/').unwrap_or_default();
        let name = suffix.trim_end_matches("_files");
        Some(name.to_string())
    }
}

impl std::fmt::Debug for DziLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{} (Deep Zoom Image)", TileProvider::title(self).unwrap_or_default())
    }
}

#[test]
fn test_panorama() {
    let url = "http://x.fr/y/test.dzi";
    let contents = br#"
        <Image
          TileSize="256"
          Overlap="2"
          Format="jpg"
          >
          <Size Width="600" Height="300"/>
          <DisplayRects></DisplayRects>
        </Image>"#;
    let mut props = load_from_properties(url, contents).unwrap();
    assert_eq!(props.len(), 11);
    let level = &mut props[1];
    let tiles: Vec<String> = level.next_tiles(None).into_iter().map(|t| t.url).collect();
    assert_eq!(
        tiles,
        vec![
            "http://x.fr/y/test_files/9/0_0.jpg",
            "http://x.fr/y/test_files/9/1_0.jpg"
        ]
    );
}


#[test]
fn test_dzi_with_bom() {
    // See https://github.com/lovasoa/dezoomify-rs/issues/45
    // Trying to parse a file with a byte order mark
    let contents = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>
        <Image TileSize=\"256\" Overlap=\"0\" Format=\"jpg\" xmlns=\"http://schemas.microsoft.com/deepzoom/2008\">
        <Size Width=\"6261\" Height=\"6047\" />
        </Image>";
    load_from_properties("http://test.com/test.xml", contents.as_ref()).unwrap();
}

#[test]
fn test_openseadragon_javascript() {
    // See https://github.com/lovasoa/dezoomify-rs/issues/45
    // Trying to parse a file with a byte order mark
    let contents = r#"OpenSeadragon({
            id:            "example-inline-configuration-for-dzi",
            prefixUrl:     "/openseadragon/images/",
            showNavigator:  true,
            tileSources:   {
                Image: {
                    xmlns:    "http://schemas.microsoft.com/deepzoom/2008",
                    Url:      "/example-images/highsmith/highsmith_files/",
                    Format:   "jpg",
                    Overlap:  "2",
                    TileSize: "256",
                    Size: {
                        Height: "9221",
                        Width:  "7026"
                    }
                }
            }
        });
    "#;
    let level =
        &mut load_from_properties("http://test.com/x/test.xml", contents.as_ref()).unwrap()[0];
    assert_eq!(Some(Vec2d { y: 9221, x: 7026 }), level.size_hint());
    let tiles: Vec<String> = level.next_tiles(None).into_iter().map(|t| t.url).collect();
    assert_eq!(tiles[0], "http://test.com/example-images/highsmith/highsmith_files/14/0_0.jpg");
}
