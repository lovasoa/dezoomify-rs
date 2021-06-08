use std::sync::Arc;

use custom_error::custom_error;
use itertools::Itertools;
use log::warn;

use krpano_metadata::{KrpanoMetadata, TemplateString, TemplateStringPart, XY};

use crate::dezoomer::*;
use crate::krpano::krpano_metadata::{ImageInfo, LevelDesc};
use crate::network::{remove_bom, resolve_relative};

mod krpano_metadata;

/// A dezoomer for krpano images
/// See https://krpano.com/docu/xml/#top
#[derive(Default)]
pub struct KrpanoDezoomer;

impl Dezoomer for KrpanoDezoomer {
    fn name(&self) -> &'static str {
        "krpano"
    }

    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        let DezoomerInputWithContents { uri, contents } = data.with_contents()?;
        let levels = load_from_properties(uri, contents)?;
        Ok(levels)
    }
}

custom_error! {pub KrpanoError
    XmlError{source: serde_xml_rs::Error} = "Unable to parse the krpano xml file: {source}",
}

impl From<KrpanoError> for DezoomerError {
    fn from(err: KrpanoError) -> Self {
        DezoomerError::Other { source: err.into() }
    }
}

fn load_from_properties(url: &str, contents: &[u8])
                        -> Result<ZoomLevels, KrpanoError> {
    let image_properties: KrpanoMetadata = serde_xml_rs::from_reader(remove_bom(contents))?;
    let base_url = &Arc::from(url);
    let title: &Arc<str> = &Arc::from(image_properties.get_title().unwrap_or(""));
    Ok(image_properties.into_image_iter().flat_map(move |ImageInfo { image, name }| {
        let root_tile_size = image.tilesize.map(Vec2d::square);
        let base_index = image.baseindex;
        image.level.into_iter().flat_map(move |level| {
            let name = Arc::clone(&name);
            level.level_descriptions(None).into_iter().flat_map(move |level_desc| {
                let name = Arc::clone(&name);
                level_desc
                    .map_err(|err| warn!("bad krpano level: {}", err))
                    .into_iter()
                    .flat_map(move |LevelDesc {
                                        name: shape_name,
                                        size,
                                        tilesize,
                                        url,
                                        level_index,
                                    }| {
                        let level = level_index + base_index as usize;
                        let name = Arc::clone(&name);
                        url.all_sides(level).flat_map(move |(side_name, template)| {
                            let base_url = Arc::clone(base_url);
                            let title = Arc::clone(title);
                            let name = Arc::clone(&name);
                            tilesize.or(root_tile_size).map(|tile_size|
                                Level {
                                    base_url,
                                    size,
                                    tile_size,
                                    base_index,
                                    template,
                                    shape_name,
                                    side_name,
                                    name,
                                    title,
                                })
                        })
                    })
            })
        })
    }).into_zoom_levels())
}

#[derive(PartialEq)]
struct Level {
    base_url: Arc<str>,
    size: Vec2d,
    tile_size: Vec2d,
    base_index: u32,
    template: TemplateString<XY>,
    shape_name: &'static str,
    side_name: &'static str,
    name: Arc<str>,
    title: Arc<str>,
}

impl TilesRect for Level {
    fn size(&self) -> Vec2d { self.size }

    fn tile_size(&self) -> Vec2d { self.tile_size }

    fn tile_url(&self, Vec2d { x, y }: Vec2d) -> String {
        use std::fmt::Write;
        let mut result = String::new();
        for part in self.template.0.iter() {
            match part {
                TemplateStringPart::Literal(s) => { result += s }
                TemplateStringPart::Variable { padding, variable } => {
                    write!(result, "{value:0padding$}",
                           value = self.base_index + match variable {
                               XY::X => x,
                               XY::Y => y
                           },
                           padding = *padding
                    ).unwrap();
                }
            }
        }
        resolve_relative(&self.base_url, &result)
    }

    fn title(&self) -> Option<String> {
        if self.title.is_empty() && self.name.is_empty() {
            None
        } else {
            let title = [self.title.as_ref(), self.name.as_ref()].join(" ");
            Some(title)
        }
    }

    fn tile_ref(&self, pos: Vec2d) -> TileReference {
        TileReference {
            url: self.tile_url(pos),
            position: self.tile_size() * pos,
        }
    }
}

impl std::fmt::Debug for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let parts = ["Krpano", self.shape_name, self.side_name, &self.name];
        write!(f, "{}", parts.iter().filter(|s| !s.is_empty()).join(" "))
    }
}

#[test]
fn test_cube() {
    let mut levels = load_from_properties(
        "http://test.com",
        r#"<krpano showerrors="false" logkey="false">
        <image type="cube" multires="true" tilesize="512" progressive="false" multiresthreshold="-0.3">
            <level download="view" decode="view" tiledimagewidth="1000" tiledimageheight="100">
                <cube url="http://example.com/%s/%r/%c.jpg"/>
            </level>
        </image>
        </krpano>"#.as_bytes(),
    ).unwrap();
    assert_eq!(levels.len(), 6);
    assert_eq!(levels[0].size_hint(), Some(Vec2d { x: 1000, y: 100 }));
    assert_eq!(format!("{:?}", levels[0]), "Krpano Cube forward");
    assert_eq!(levels[0].next_tiles(None), vec![
        TileReference { url: "http://example.com/f/1/1.jpg".to_string(), position: Vec2d { x: 0, y: 0 } },
        TileReference { url: "http://example.com/f/1/2.jpg".to_string(), position: Vec2d { x: 512, y: 0 } }]);
}

#[test]
fn test_flat_multires() {
    let mut levels = load_from_properties(
        "http://test.com",
        r#"<krpano>
        <image>
            <flat url="level=%l x=%0x y=%0y" multires="1,2x3,3x4x3"/>
        </image>
        </krpano>"#.as_bytes(),
    ).unwrap();
    assert_eq!(levels.len(), 2);
    assert_eq!(levels[1].size_hint(), Some(Vec2d { x: 3, y: 4 }));
    assert_eq!(format!("{:?}", levels[0]), "Krpano Flat");
    assert_eq!(levels[1].next_tiles(None), vec![
        TileReference { url: "http://test.com/level=2%20x=01%20y=01".to_string(), position: Vec2d { x: 0, y: 0 } },
        TileReference { url: "http://test.com/level=2%20x=01%20y=02".to_string(), position: Vec2d { x: 0, y: 3 } }]);
}