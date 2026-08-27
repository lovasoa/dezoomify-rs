use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, de};

use crate::Vec2d;
use crate::template::{Part, Template, push_padded};

#[derive(Debug, Deserialize, Default)]
pub struct KrpanoMetadata {
    #[serde(default)]
    image: Vec<KrpanoImage>,
    #[serde(default)]
    scene: Vec<KrpanoMetadata>,
    #[serde(default)]
    data: Vec<String>,
    #[serde(default)]
    source_details: Vec<SourceDetails>,

    // Actions contain krpano scripts, not tile URL metadata.
    #[serde(default, rename = "action")]
    _action: Vec<de::IgnoredAny>,

    // Events only bind scripts to viewer lifecycle hooks.
    #[serde(default, rename = "events")]
    _events: Vec<de::IgnoredAny>,

    // Includes may add more metadata at runtime, but this parser only handles
    // the XML document it was given and cannot fetch arbitrary tour UI files.
    #[serde(default, rename = "include")]
    _include: Vec<de::IgnoredAny>,

    // Nested krpano elements set global viewer variables, not image levels.
    #[serde(default, rename = "krpano")]
    _krpano: Vec<de::IgnoredAny>,

    // Security/cross-domain declarations do not affect tile geometry or URLs.
    #[serde(default, rename = "security")]
    _security: Vec<de::IgnoredAny>,

    #[serde(default, rename = "@name")]
    name: String,
}

#[derive(Debug, Deserialize)]
struct SourceDetails {
    #[serde(default, rename = "@subject")]
    subject: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ImageInfo {
    pub image: KrpanoImage,
    pub name: Arc<str>,
}

impl KrpanoMetadata {
    #[cfg(test)]
    fn from_str(s: &str) -> Result<Self, serde_xml_rs::Error> {
        serde_xml_rs::SerdeXml::new()
            .overlapping_sequences(true)
            .from_str(s)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_xml_rs::Error> {
        serde_xml_rs::SerdeXml::new()
            .overlapping_sequences(true)
            .from_reader(bytes)
    }

    fn into_image_iter_with_name(self, parent_name: &str) -> Box<dyn Iterator<Item = ImageInfo>> {
        let name: Arc<str> = if parent_name.is_empty() {
            Arc::from(self.name)
        } else {
            let s = [parent_name, &self.name].join(" ");
            Arc::from(s)
        };
        let images = self
            .image
            .into_iter()
            .filter(KrpanoImage::has_tile_levels)
            .map({
                let name = Arc::clone(&name);
                move |image| ImageInfo {
                    image,
                    name: Arc::clone(&name),
                }
            });
        let scene_images = self
            .scene
            .into_iter()
            .flat_map(move |s| s.into_image_iter_with_name(&name));
        Box::new(images.chain(scene_images))
    }

    pub fn into_image_iter(self) -> impl Iterator<Item = ImageInfo> {
        self.into_image_iter_with_name("")
    }

    pub fn get_title(&self) -> Option<&str> {
        self.source_details
            .iter()
            .find_map(|details| (!details.subject.is_empty()).then_some(details.subject.as_str()))
            .or_else(|| {
                self.data.iter().find_map(|bytes| {
                    serde_json::from_str::<KrpanoMetaData>(bytes)
                        .ok()
                        .map(|m| m.title)
                })
            })
    }
}

#[derive(Deserialize)]
struct KrpanoMetaData<'a> {
    title: &'a str,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Default)]
pub struct KrpanoImage {
    #[serde(rename = "@tilesize")]
    pub tilesize: Option<u32>,
    #[serde(default = "default_base_index", rename = "@baseindex")]
    pub baseindex: u32,
    #[serde(default, rename = "#content")]
    pub level: Vec<KrpanoLevel>,
    #[serde(default)]
    pub cube: Vec<ShapeDesc>,
    #[serde(default)]
    pub cylinder: Vec<ShapeDesc>,
    #[serde(default)]
    pub flat: Vec<ShapeDesc>,
    #[serde(default)]
    pub sphere: Vec<ShapeDesc>,
    #[serde(default)]
    pub left: Vec<ShapeDesc>,
    #[serde(default)]
    pub right: Vec<ShapeDesc>,
    #[serde(default)]
    pub front: Vec<ShapeDesc>,
    #[serde(default)]
    pub back: Vec<ShapeDesc>,
    #[serde(default)]
    pub up: Vec<ShapeDesc>,
    #[serde(default)]
    pub down: Vec<ShapeDesc>,
}

impl KrpanoImage {
    pub fn into_levels(self) -> impl Iterator<Item = KrpanoLevel> {
        self.into_all_levels()
    }

    pub fn has_tile_levels(&self) -> bool {
        !self.level.is_empty()
            || self.cube.iter().any(|shape| shape.multires.is_some())
            || self.cylinder.iter().any(|shape| shape.multires.is_some())
            || self.flat.iter().any(|shape| shape.multires.is_some())
            || self.sphere.iter().any(|shape| shape.multires.is_some())
            || self.left.iter().any(|shape| shape.multires.is_some())
            || self.right.iter().any(|shape| shape.multires.is_some())
            || self.front.iter().any(|shape| shape.multires.is_some())
            || self.back.iter().any(|shape| shape.multires.is_some())
            || self.up.iter().any(|shape| shape.multires.is_some())
            || self.down.iter().any(|shape| shape.multires.is_some())
    }

    fn into_all_levels(self) -> impl Iterator<Item = KrpanoLevel> {
        self.level
            .into_iter()
            .chain(self.cube.into_iter().map(KrpanoLevel::Cube))
            .chain(self.cylinder.into_iter().map(KrpanoLevel::Cylinder))
            .chain(self.flat.into_iter().map(KrpanoLevel::Flat))
            .chain(self.sphere.into_iter().map(KrpanoLevel::Sphere))
            .chain(self.left.into_iter().map(KrpanoLevel::Left))
            .chain(self.right.into_iter().map(KrpanoLevel::Right))
            .chain(self.front.into_iter().map(KrpanoLevel::Front))
            .chain(self.back.into_iter().map(KrpanoLevel::Back))
            .chain(self.up.into_iter().map(KrpanoLevel::Up))
            .chain(self.down.into_iter().map(KrpanoLevel::Down))
    }
}

fn default_base_index() -> u32 {
    1
}

pub struct LevelDesc {
    pub name: &'static str,
    pub size: Vec2d,
    pub tilesize: Option<Vec2d>,
    pub url: Template<TemplateVariable>,
    pub level_index: usize,
}

#[derive(Deserialize, PartialEq, Eq, Debug)]
pub struct ShapeDesc {
    #[serde(rename = "@url")]
    url: Template<TemplateVariable>,
    #[serde(rename = "@multires")]
    multires: Option<String>,
}

#[derive(Deserialize, PartialEq, Eq, Debug)]
pub struct LevelAttributes {
    #[serde(rename = "@tiledimagewidth")]
    tiledimagewidth: u32,
    #[serde(rename = "@tiledimageheight")]
    tiledimageheight: u32,
    #[serde(rename = "#content")]
    shape: Vec<KrpanoLevel>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KrpanoLevel {
    Level(LevelAttributes),
    Mobile(Vec<KrpanoLevel>),
    Tablet(Vec<KrpanoLevel>),
    Cube(ShapeDesc),
    Cylinder(ShapeDesc),
    Flat(ShapeDesc),
    Sphere(ShapeDesc),
    Left(ShapeDesc),
    Right(ShapeDesc),
    Front(ShapeDesc),
    Back(ShapeDesc),
    Up(ShapeDesc),
    Down(ShapeDesc),
}

impl KrpanoLevel {
    pub fn level_descriptions(
        self,
        size: Option<Vec2d>,
        level_index: usize,
    ) -> Vec<Result<LevelDesc, &'static str>> {
        match self {
            Self::Level(LevelAttributes {
                tiledimagewidth,
                tiledimageheight,
                shape,
            }) => {
                let size = Vec2d {
                    x: tiledimagewidth,
                    y: tiledimageheight,
                };
                shape
                    .into_iter()
                    .flat_map(|level| level.level_descriptions(Some(size), level_index))
                    .collect()
            }
            Self::Cube(d) => shape_descriptions("Cube", d, size, level_index),
            Self::Cylinder(d) => shape_descriptions("Cylinder", d, size, level_index),
            Self::Flat(d) => shape_descriptions("Flat", d, size, level_index),
            Self::Sphere(d) => shape_descriptions("Sphere", d, size, level_index),
            Self::Left(d) => shape_descriptions("Left", d, size, level_index),
            Self::Right(d) => shape_descriptions("Right", d, size, level_index),
            Self::Front(d) => shape_descriptions("Front", d, size, level_index),
            Self::Back(d) => shape_descriptions("Back", d, size, level_index),
            Self::Up(d) => shape_descriptions("Up", d, size, level_index),
            Self::Down(d) => shape_descriptions("Down", d, size, level_index),
            Self::Mobile(_) | Self::Tablet(_) => vec![], // Ignore
        }
    }
}

fn shape_descriptions(
    name: &'static str,
    desc: ShapeDesc,
    size: Option<Vec2d>,
    level_index: usize,
) -> Vec<Result<LevelDesc, &'static str>> {
    let ShapeDesc { multires, url } = desc;
    if let Some(multires) = multires {
        parse_multires(&multires)
            .enumerate()
            .map(|(level_index, result)| {
                result.map(|(size, tilesize)| LevelDesc {
                    name,
                    size,
                    tilesize: Some(tilesize),
                    url: url.clone(),
                    level_index,
                })
            })
            .collect()
    } else if let Some(size) = size {
        let tilesize = None;
        vec![Ok(LevelDesc {
            name,
            size,
            tilesize,
            url,
            level_index,
        })]
    } else {
        vec![Err("missing multires attribute")]
    }
}

/// Parse a multires string into a vector of (image size, `tile_size`)
fn parse_multires(s: &str) -> impl Iterator<Item = Result<(Vec2d, Vec2d), &'static str>> + '_ {
    let mut parts = s.split(',');
    let tilesize_x: Result<u32, _> = parts
        .next()
        .and_then(|x| x.parse().ok())
        .ok_or("missing tile size");
    parts.map(move |dim_str| {
        tilesize_x.and_then(|tilesize_x| {
            let mut dims = dim_str.split('x');
            let x: u32 = dims
                .next()
                .ok_or("missing width")?
                .parse()
                .map_err(|_| "invalid width")?;
            let y: u32 = dims.next().and_then(|x| x.parse().ok()).unwrap_or(x);
            let tilesize = dims
                .next()
                .and_then(|x| x.parse().ok())
                .unwrap_or(tilesize_x);
            Ok((Vec2d { x, y }, Vec2d::square(tilesize)))
        })
    })
}

impl<'de> Deserialize<'de> for Template<TemplateVariable> {
    fn deserialize<D>(deserializer: D) -> Result<Self, <D as Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
    {
        use de::Error;
        String::deserialize(deserializer)?
            .parse()
            .map_err(Error::custom)
    }
}

impl FromStr for Template<TemplateVariable> {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        use Part::{Hole, Literal};
        use TemplateVariable::{LevelIndex, Side, X, Y};
        use itertools::Itertools;
        let mut chars = input.chars();
        let mut parts = vec![];
        loop {
            let literal: String = chars.take_while_ref(|&c| c != '%').collect();
            if !literal.is_empty() {
                parts.push(Literal(Arc::from(literal)));
            }
            if chars.next().is_none() {
                break;
            }
            let padding = 1 + chars.take_while_ref(|&c| c == '0').count();
            parts.push(match chars.next() {
                Some('h' | 'x' | 'u' | 'c') => Hole(X, padding),
                Some('v' | 'y' | 'r') => Hole(Y, padding),
                Some('s') => Hole(Side, padding),
                Some('l') => Hole(LevelIndex, padding),
                Some('%') => Literal(Arc::from("%")),
                Some(x) => return Err(format!("Unknown template variable '{x}' in '{input}'")),
                None => return Err(format!("Invalid templating syntax in '{input}'")),
            });
        }
        Ok(Template(parts))
    }
}

pub fn all_sides(
    template: Template<TemplateVariable>,
    level: usize,
) -> impl Iterator<Item = (&'static str, Template<XY>)> + 'static {
    let has_side = template
        .0
        .iter()
        .any(|part| matches!(part, Part::Hole(TemplateVariable::Side, _)));
    let sides = if has_side {
        &["forward", "back", "left", "right", "up", "down"][..]
    } else {
        &[""]
    };
    sides.iter().map(move |&side| {
        (
            side,
            Template(
                template
                    .0
                    .iter()
                    .map(|part| part.with_side(side, level))
                    .collect(),
            ),
        )
    })
}

impl Part<TemplateVariable> {
    fn with_side(&self, side: &'static str, level: usize) -> Part<XY> {
        use Part::{Hole, Literal};
        use TemplateVariable::{LevelIndex, Side, X, Y};
        match self {
            Literal(s) => Literal(Arc::clone(s)),
            Hole(variable, padding) => {
                let padding = *padding;
                match variable {
                    X => Hole(XY::X, padding),
                    Y => Hole(XY::Y, padding),
                    Side => Literal(Arc::from(&side[..1])),
                    LevelIndex => {
                        let mut idx_str = String::new();
                        push_padded(&mut idx_str, level, padding);
                        Literal(Arc::from(idx_str))
                    }
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TemplateVariable {
    X,
    Y,
    Side,
    LevelIndex,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum XY {
    X,
    Y,
}

#[cfg(test)]
mod test {
    use super::KrpanoLevel::{Cube, Cylinder, Left, Mobile};
    use super::TemplateVariable::{LevelIndex, X, Y};
    use super::*;
    use crate::template::Part::{Hole, Literal};

    fn str(s: &str) -> Part<TemplateVariable> {
        Literal(Arc::from(s))
    }

    fn x(padding: usize) -> Part<TemplateVariable> {
        Hole(X, padding)
    }

    fn y(padding: usize) -> Part<TemplateVariable> {
        Hole(Y, padding)
    }

    fn lvl(padding: usize) -> Part<TemplateVariable> {
        Hole(LevelIndex, padding)
    }

    #[test]
    fn parse_xml_cylinder() {
        let parsed = KrpanoMetadata::from_str(r#"
        <krpano version="1.18"  bgcolor="0xFFFFFF">
            <include url="skin/flatpano_setup.xml" />
            <view devices="mobile" hlookat="0" vlookat="0" maxpixelzoom="0.7" limitview="fullrange" fov="1.8" fovmax="1.8" fovmin="0.02"/>
            <preview url="monomane.tiles/preview.jpg" />
            <image type="CYLINDER" hfov="1.00" vfov="1.208146" voffset="0.00" multires="true" tilesize="512" progressive="true">
                <level tiledimagewidth="31646" tiledimageheight="38234">
                    <cylinder url="monomane.tiles/l7/%v/l7_%v_%h.jpg" />
                </level>
            </image>
        </krpano>
        "#).unwrap();
        let images: Vec<ImageInfo> = parsed.into_image_iter().collect();
        assert_eq!(
            images,
            vec![ImageInfo {
                name: Arc::from(""),
                image: KrpanoImage {
                    baseindex: 1,
                    tilesize: Some(512),
                    level: vec![KrpanoLevel::Level(LevelAttributes {
                        tiledimagewidth: 31646,
                        tiledimageheight: 38234,
                        shape: vec![KrpanoLevel::Cylinder(ShapeDesc {
                            url: Template(vec![
                                str("monomane.tiles/l7/"),
                                y(1),
                                str("/l7_"),
                                y(1),
                                str("_"),
                                x(1),
                                str(".jpg"),
                            ]),
                            multires: None,
                        })],
                    }),],
                    ..Default::default()
                },
            }]
        );
    }

    #[test]
    fn get_title_json_metadata() {
        let parsed = KrpanoMetadata::from_str(
            r#"
        <krpano version="1.18"  bgcolor="0xFFFFFF">
            <data name="metadata"><![CDATA[
                {"id":"xxx", "title":"yyy"}
            ]]></data>
        </krpano>
        "#,
        )
        .unwrap();
        assert_eq!(parsed.get_title(), Some("yyy"));
    }

    #[test]
    fn get_title_source_details() {
        let parsed = KrpanoMetadata::from_str(
            r#"
        <krpano version="1.18"  bgcolor="0xFFFFFF">
            <source_details subject="the subject"/>
        </krpano>
        "#,
        )
        .unwrap();
        assert_eq!(parsed.get_title(), Some("the subject"));
    }

    #[test]
    fn parse_xml_old_cube() {
        let parsed = KrpanoMetadata::from_str(r#"<krpano showerrors="false" logkey="false">
        <image type="cube" multires="true" tilesize="512" baseindex="0" progressive="false" multiresthreshold="-0.3">
            <level download="view" decode="view" tiledimagewidth="3280" tiledimageheight="3280">
                <left  url="https://example.com/%000r/%0000c.jpg"/>
            </level>
        </image>
        </krpano>"#).unwrap();
        let images: Vec<ImageInfo> = parsed.into_image_iter().collect();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].image.baseindex, 0);
        assert_eq!(images[0].image.tilesize, Some(512));
        assert_eq!(
            images[0].image.level,
            vec![KrpanoLevel::Level(LevelAttributes {
                tiledimagewidth: 3280,
                tiledimageheight: 3280,
                shape: vec![Left(ShapeDesc {
                    url: Template(vec![
                        str("https://example.com/"),
                        y(4),
                        str("/"),
                        x(5),
                        str(".jpg")
                    ]),
                    multires: None,
                })],
            })]
        );
    }

    #[test]
    fn parse_xml_multires() {
        let parsed = KrpanoMetadata::from_str(r#"
        <krpano>
        <image>
            <flat url="https://example.com/" multires="512,768x554,1664x1202,3200x2310,6400x4618,12800x9234"/>
        </image>
        </krpano>"#).unwrap();
        let mut images = parsed.into_image_iter();
        let image = images.next().unwrap().image;
        assert!(images.next().is_none());
        assert_eq!(image.baseindex, 1);
        assert_eq!(image.tilesize, None);
        assert_eq!(
            image.into_levels().collect::<Vec<_>>(),
            vec![KrpanoLevel::Flat(ShapeDesc {
                url: Template(vec![str("https://example.com/"),]),
                multires: Some("512,768x554,1664x1202,3200x2310,6400x4618,12800x9234".to_string()),
            })]
        );
    }

    #[test]
    fn parse_xml_mobile() {
        // See https://github.com/lovasoa/dezoomify-rs/issues/58
        let parsed = KrpanoMetadata::from_str(
            r#"
        <krpano>
        <image>
            <mobile>
                <cube url="test.jpg" />
            </mobile>
        </image>
        </krpano>"#,
        )
        .unwrap();
        let images: Vec<ImageInfo> = parsed.into_image_iter().collect();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].image.baseindex, 1);
        assert_eq!(images[0].image.tilesize, None);
        assert_eq!(
            images[0].image.level,
            vec![Mobile(vec![Cube(ShapeDesc {
                url: Template(vec![str("test.jpg")]),
                multires: None,
            })])]
        );
    }

    #[test]
    fn parse_xml_with_scene() {
        // See https://github.com/lovasoa/dezoomify-rs/issues/100#issuecomment-767048175
        let parsed = KrpanoMetadata::from_str(r#"<krpano version="1.18">
        <scene name="scene_Color">
            <image type="CYLINDER" hfov="1.00" vfov="1.291661" voffset="0.00" multires="true" tilesize="512">
                <level tiledimagewidth="7424" tiledimageheight="9590">
                    <cylinder url="xxx/%0v/l5_%0v_%0h.jpg"/>
                </level>
            </image>
        </scene>
        </krpano>"#).unwrap();
        let images: Vec<ImageInfo> = parsed.into_image_iter().collect();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].name.as_ref(), "scene_Color");
        assert_eq!(images[0].image.baseindex, 1);
        assert_eq!(images[0].image.tilesize, Some(512));
        assert_eq!(
            images[0].image.level,
            vec![KrpanoLevel::Level(LevelAttributes {
                tiledimagewidth: 7424,
                tiledimageheight: 9590,
                shape: vec![Cylinder(ShapeDesc {
                    url: Template(vec![
                        str("xxx/"),
                        y(2),
                        str("/l5_"),
                        y(2),
                        str("_"),
                        x(2),
                        str(".jpg")
                    ]),
                    multires: None,
                })],
            })]
        );
    }

    #[test]
    fn parse_panotour_xml_with_interleaved_unknown_tags() {
        let parsed = KrpanoMetadata::from_str(
            r#"<krpano version="1.19">
            <security><allowdomain domain="*" /></security>
            <events name="startbehavioursevents" />
            <include url="%FIRSTXML%/index_skin.xml" />
            <scene name="pano1">
                <autorotate speed="5" />
                <preview url="pano1/preview.jpg" type="CYLINDER" />
                <image type="CYLINDER" multires="true" baseindex="0" tilesize="512">
                    <level tiledimagewidth="25000" tiledimageheight="15431">
                        <cylinder url="pano1/4/%v/%u.jpg" />
                    </level>
                </image>
                <action name="ignored">noop();</action>
            </scene>
            <scene name="pano2">
                <image type="CUBE" multires="true" baseindex="0" tilesize="512">
                    <level tiledimagewidth="1024" tiledimageheight="1024">
                        <front url="pano2/0/%v_%u.jpg" />
                    </level>
                </image>
            </scene>
            <krpano nofullspherepanoavailable="false" />
        </krpano>"#,
        )
        .unwrap();

        let infos: Vec<ImageInfo> = parsed.into_image_iter().collect();
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].name.as_ref(), "pano1");
        assert_eq!(infos[1].name.as_ref(), "pano2");
    }

    #[test]
    fn parse_factum_arte() {
        // See https://github.com/lovasoa/dezoomify-rs/issues/100#issuecomment-767048175
        let bytes = std::fs::read("testdata/krpano/krpano_scenes.xml").unwrap();
        let parsed = KrpanoMetadata::from_bytes(&bytes).unwrap();
        let infos: Vec<ImageInfo> = parsed.into_image_iter().collect();
        assert_eq!(infos.len(), 3);
        let names: Vec<String> = infos
            .iter()
            .map(|i| String::from(i.name.as_ref()))
            .collect();
        assert_eq!(names, ["scene_Color", "scene_3D", "scene_3Dcolor"]);
    }

    #[test]
    fn parse_360cities() {
        // title: St George Hotel Dubai Tip Top English Disco by 360emirates
        let bytes = std::fs::read("testdata/krpano/krpano_360cities.xml").unwrap();
        let parsed = KrpanoMetadata::from_bytes(&bytes).unwrap();
        let infos: Vec<ImageInfo> = parsed.into_image_iter().collect();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].image.level.len(), 4);
    }

    #[test]
    fn parse_geografiche_panotour() {
        let bytes = std::fs::read("testdata/krpano/geografiche.xml").unwrap();
        let parsed = KrpanoMetadata::from_bytes(&bytes).unwrap();
        let infos: Vec<ImageInfo> = parsed.into_image_iter().collect();
        assert_eq!(infos.len(), 13);
        assert_eq!(infos[0].name.as_ref(), "pano23128");
    }

    #[test]
    fn multires_parse() {
        let expected: Vec<Result<_, &'static str>> = vec![
            Ok((Vec2d { x: 6, y: 7 }, Vec2d { x: 3, y: 3 })),
            Ok((Vec2d { x: 8, y: 8 }, Vec2d { x: 3, y: 3 })),
            Ok((Vec2d { x: 9, y: 1 }, Vec2d { x: 4, y: 4 })),
        ];
        assert_eq!(
            expected,
            parse_multires("3,6x7,8x8,9x1x4").collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_templatestring() {
        assert_eq!(
            Ok(Template(vec![x(3), str("%"), y(2), lvl(1)])),
            "%00x%%%0y%l".parse()
        );
    }
}
