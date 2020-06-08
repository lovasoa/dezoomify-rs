use std::sync::Arc;

use custom_error::custom_error;

use crate::dezoomer::*;
use crate::krpano::krpano_metadata::{KrpanoMetadata, Shape, TemplateString, TemplateStringPart, XY};
use crate::network::{resolve_relative, remove_bom};
use itertools::Itertools;

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
    let slash_pos = url.rfind('/').unwrap_or(url.len() - 1);
    let base_url = &Arc::new(format!("{}/", &url[0..slash_pos]));

    Ok(image_properties.image.into_iter().flat_map(move |image| {
        let tile_size = Vec2d { x: image.tilesize, y: image.tilesize };
        let base_index = image.baseindex;
        image.level.into_iter().flat_map(move |level| {
            let size = Vec2d { x: level.tiledimagewidth, y: level.tiledimageheight };
            level.shape.into_iter().flat_map(move |shape: Shape| {
                let (shape_name, template) = shape.name_and_url();
                template.all_sides().map(move |(side_name, template)| {
                    let base_url = Arc::clone(base_url);
                    Level {
                        base_url,
                        base_index,
                        size,
                        tile_size,
                        template,
                        shape_name,
                        side_name,
                    }
                })
            })
        })
    }).into_zoom_levels())
}

#[derive(PartialEq)]
struct Level {
    base_url: Arc<String>,
    size: Vec2d,
    tile_size: Vec2d,
    base_index: u32,
    template: TemplateString<XY>,
    shape_name: &'static str,
    side_name: &'static str,
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
                           padding = *padding as usize
                    ).unwrap();
                }
            }
        }
        resolve_relative(&self.base_url, &result)
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
        let parts = ["Krpano", self.shape_name, self.side_name];
        write!(f, "{}", parts.iter().filter(|s| !s.is_empty()).join(" "))
    }
}

#[test]
fn test() {
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