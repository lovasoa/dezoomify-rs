use std::str::FromStr;
use std::sync::Arc;

use serde::{de, Deserialize, Deserializer};

use crate::Vec2d;

#[derive(Debug, Deserialize, PartialEq, Eq, Default)]
pub struct KrpanoMetadata {
    #[serde(rename = "$value")]
    children: Vec<TopLevelTags>,
    #[serde(default)]
    name: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ImageInfo {
    pub image: KrpanoImage,
    pub name: Arc<str>,
}

impl KrpanoMetadata {
    fn into_image_iter_with_name(self, name: Arc<str>) -> impl Iterator<Item=ImageInfo> {
        let name: Arc<str> = if name.is_empty() {
            Arc::from(self.name)
        } else {
            let s = [name.as_ref(), &self.name].join(" ");
            Arc::from(s)
        };
        self.children.into_iter()
            .flat_map(move |t| t.into_image_iter_with_name(name.clone()))
    }

    pub fn into_image_iter(self) -> impl Iterator<Item=ImageInfo> {
        self.into_image_iter_with_name(Arc::from(""))
    }

    pub fn get_title(&self) -> Option<&str> {
        self.children.iter().find_map(|child| child.get_title())
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TopLevelTags {
    Image(KrpanoImage),
    Scene(KrpanoMetadata),
    SourceDetails {
        #[serde(default)] subject: String,
    },
    Data(String),
    #[serde(other, deserialize_with = "deserialize_ignore_any")]
    Other,
}


fn deserialize_ignore_any<'de, D: Deserializer<'de>>(deserializer: D) -> Result<(), D::Error> {
    serde::de::IgnoredAny::deserialize(deserializer)?;
    Ok(())
}

impl TopLevelTags {
    fn into_image_iter_with_name(self, name: Arc<str>) -> Box<dyn Iterator<Item=ImageInfo>> {
        match self {
            Self::Image(image) =>
                Box::new(std::iter::once(ImageInfo { image, name })),
            Self::Scene(s) =>
                Box::new(s.into_image_iter_with_name(name)),
            _ =>
                Box::new(std::iter::empty())
        }
    }
    fn get_title(&self) -> Option<&str> {
        match self {
            Self::SourceDetails { subject } => Some(subject),
            Self::Data(bytes) =>
                serde_json::from_str::<KrpanoMetaData>(bytes).ok()
                    .map(|m| m.title),
            _ => None
        }
    }
}

#[derive(Deserialize)]
struct KrpanoMetaData<'a> {
    title: &'a str
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct KrpanoImage {
    pub tilesize: Option<u32>,
    #[serde(default = "default_base_index")]
    pub baseindex: u32,
    #[serde(rename = "$value")]
    pub level: Vec<KrpanoLevel>,
}

fn default_base_index() -> u32 { 1 }

pub struct LevelDesc {
    pub name: &'static str,
    pub size: Vec2d,
    pub tilesize: Option<Vec2d>,
    pub url: TemplateString<TemplateVariable>,
    pub level_index: usize,
}

#[derive(Deserialize, PartialEq, Eq, Debug)]
pub struct ShapeDesc {
    url: TemplateString<TemplateVariable>,
    multires: Option<String>,
}

#[derive(Deserialize, PartialEq, Eq, Debug)]
pub struct LevelAttributes {
    tiledimagewidth: u32,
    tiledimageheight: u32,
    #[serde(rename = "$value")]
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
    Left(ShapeDesc),
    Right(ShapeDesc),
    Front(ShapeDesc),
    Back(ShapeDesc),
    Up(ShapeDesc),
    Down(ShapeDesc),
}

impl KrpanoLevel {
    pub fn level_descriptions(self, size: Option<Vec2d>) -> Vec<Result<LevelDesc, &'static str>> {
        match self {
            Self::Level(LevelAttributes { tiledimagewidth, tiledimageheight, shape }) => {
                let size = Vec2d { x: tiledimagewidth, y: tiledimageheight };
                shape.into_iter().flat_map(|level| level.level_descriptions(Some(size))).collect()
            }
            Self::Cube(d) => shape_descriptions("Cube", d, size),
            Self::Cylinder(d) => shape_descriptions("Cylinder", d, size),
            Self::Flat(d) => shape_descriptions("Flat", d, size),
            Self::Left(d) => shape_descriptions("Left", d, size),
            Self::Right(d) => shape_descriptions("Right", d, size),
            Self::Front(d) => shape_descriptions("Front", d, size),
            Self::Back(d) => shape_descriptions("Back", d, size),
            Self::Up(d) => shape_descriptions("Up", d, size),
            Self::Down(d) => shape_descriptions("Down", d, size),
            Self::Mobile(_) | Self::Tablet(_) => vec![], // Ignore
        }
    }
}

fn shape_descriptions(
    name: &'static str,
    desc: ShapeDesc,
    size: Option<Vec2d>,
) -> Vec<Result<LevelDesc, &'static str>> {
    let ShapeDesc { multires, url } = desc;
    if let Some(multires) = multires {
        parse_multires(&multires).enumerate().map(|(level_index, result)|
            result.map(|(size, tilesize)| LevelDesc {
                name,
                size,
                tilesize: Some(tilesize),
                url: url.clone(),
                level_index,
            })
        ).collect()
    } else if let Some(size) = size {
        let tilesize = None;
        vec![Ok(LevelDesc { name, size, tilesize, url, level_index: 0 })]
    } else {
        vec![Err("missing multires attribute")]
    }
}

/// Parse a multires string into a vector of (image size, tile_size)
fn parse_multires(s: &str) -> impl Iterator<Item=Result<(Vec2d, Vec2d), &'static str>> + '_ {
    let mut parts = s.split(',');
    let tilesize_x: Result<u32, _> = parts.next()
        .and_then(|x| x.parse().ok())
        .ok_or("missing tile size");
    parts.map(move |dim_str| {
        tilesize_x.and_then(|tilesize_x| {
            let mut dims = dim_str.split('x');
            let x: u32 = dims
                .next().ok_or("missing width")?
                .parse().map_err(|_| "invalid width")?;
            let y: u32 = dims
                .next().and_then(|x| x.parse().ok())
                .unwrap_or(x);
            let tilesize = dims.next()
                .and_then(|x| x.parse().ok())
                .unwrap_or(tilesize_x);
            Ok((Vec2d { x, y }, Vec2d::square(tilesize)))
        })
    })
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TemplateString<T>(pub Vec<TemplateStringPart<T>>);

impl<'de> Deserialize<'de> for TemplateString<TemplateVariable> {
    fn deserialize<D>(deserializer: D) -> Result<Self, <D as Deserializer<'de>>::Error> where
        D: Deserializer<'de> {
        use de::Error;
        String::deserialize(deserializer)?.parse().map_err(Error::custom)
    }
}


impl FromStr for TemplateString<TemplateVariable> {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        use itertools::Itertools;
        use TemplateStringPart::*;
        use TemplateVariable::*;
        let mut chars = input.chars();
        let mut parts = vec![];
        loop {
            let literal: String = chars.take_while_ref(|&c| c != '%').collect();
            if !literal.is_empty() {
                parts.push(Literal(Arc::from(literal)));
            }
            if chars.next().is_none() { break; }
            let padding = 1 + chars.take_while_ref(|&c| c == '0').count();
            parts.push(match chars.next() {
                Some('h') | Some('x') | Some('u') | Some('c') => Variable { padding, variable: X },
                Some('v') | Some('y') | Some('r') => Variable { padding, variable: Y },
                Some('s') => Variable { padding, variable: Side },
                Some('l') => Variable { padding, variable: LevelIndex },
                Some('%') => Literal(Arc::from("%")),
                Some(x) => return Err(format!("Unknown template variable '{}' in '{}'", x, input)),
                None => return Err(format!("Invalid templating syntax in '{}'", input))
            });
        }
        Ok(TemplateString(parts))
    }
}

impl TemplateString<TemplateVariable> {
    pub fn all_sides(self, level: usize) -> impl Iterator<Item=(&'static str, TemplateString<XY>)> + 'static {
        let has_side = self.0.iter().any(|x| match x {
            TemplateStringPart::Variable { variable, .. } => *variable == TemplateVariable::Side,
            _ => false
        });
        let sides = if has_side { &["forward", "back", "left", "right", "up", "down"][..] } else { &[""] };
        sides.iter().map(move |&side| (side, TemplateString(
            self.0.iter().map(|part| {
                part.with_side(side, level)
            }).collect()
        )))
    }
}


#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TemplateStringPart<T> {
    Literal(Arc<str>),
    Variable { padding: usize, variable: T },
}

impl TemplateStringPart<TemplateVariable> {
    fn with_side(&self, side: &'static str, level: usize) -> TemplateStringPart<XY> {
        use TemplateStringPart::*;
        use TemplateVariable::*;
        match self {
            Literal(s) => Literal(Arc::clone(s)),
            Variable { padding, variable } => {
                let padding = *padding;
                match variable {
                    X => Variable { padding, variable: XY::X },
                    Y => Variable { padding, variable: XY::Y },
                    Side => Literal(Arc::from(&side[..1])),
                    LevelIndex => {
                        let idx_str = format!("{v:0padding$}", v = level, padding = padding);
                        Literal(Arc::from(idx_str))
                    }
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TemplateVariable { X, Y, Side, LevelIndex }

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum XY { X, Y }

#[cfg(test)]
mod test {
    use super::*;
    use super::KrpanoLevel::{Cube, Cylinder, Left, Mobile};
    use super::TemplateStringPart::{Literal, Variable};
    use super::TemplateVariable::{LevelIndex, X, Y};
    use super::TopLevelTags::{Image, Scene};

    fn str(s: &str) -> TemplateStringPart<TemplateVariable> { Literal(Arc::from(s)) }

    fn x(padding: usize) -> TemplateStringPart<TemplateVariable> { Variable { padding, variable: X } }

    fn y(padding: usize) -> TemplateStringPart<TemplateVariable> { Variable { padding, variable: Y } }

    fn lvl(padding: usize) -> TemplateStringPart<TemplateVariable> { Variable { padding, variable: LevelIndex } }

    #[test]
    fn parse_xml_cylinder() {
        let parsed: KrpanoMetadata = serde_xml_rs::from_str(r#"
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
        assert_eq!(images, vec![
            ImageInfo {
                name: Arc::from(""),
                image: KrpanoImage {
                    baseindex: 1,
                    tilesize: Some(512),
                    level: vec![
                        KrpanoLevel::Level(LevelAttributes {
                            tiledimagewidth: 31646,
                            tiledimageheight: 38234,
                            shape: vec![KrpanoLevel::Cylinder(ShapeDesc {
                                url: TemplateString(vec![
                                    str("monomane.tiles/l7/"), y(1), str("/l7_"),
                                    y(1), str("_"), x(1), str(".jpg"),
                                ]),
                                multires: None,
                            })],
                        }),
                    ],
                },
            }]);
    }

    #[test]
    fn get_title_json_metadata() {
        let parsed: KrpanoMetadata = serde_xml_rs::from_str(r#"
        <krpano version="1.18"  bgcolor="0xFFFFFF">
            <data name="metadata"><![CDATA[
                {"id":"xxx", "title":"yyy"}
            ]]></data>
        </krpano>
        "#).unwrap();
        assert_eq!(parsed.get_title(), Some("yyy"));
    }

    #[test]
    fn get_title_source_details() {
        let parsed: KrpanoMetadata = serde_xml_rs::from_str(r#"
        <krpano version="1.18"  bgcolor="0xFFFFFF">
            <source_details subject="the subject"/>
        </krpano>
        "#).unwrap();
        assert_eq!(parsed.get_title(), Some("the subject"));
    }

    #[test]
    fn parse_xml_old_cube() {
        let parsed: KrpanoMetadata = serde_xml_rs::from_str(r#"<krpano showerrors="false" logkey="false">
        <image type="cube" multires="true" tilesize="512" baseindex="0" progressive="false" multiresthreshold="-0.3">
            <level download="view" decode="view" tiledimagewidth="3280" tiledimageheight="3280">
                <left  url="https://example.com/%000r/%0000c.jpg"/>
            </level>
        </image>
        </krpano>"#).unwrap();
        assert_eq!(parsed, KrpanoMetadata {
            children: vec![Image(KrpanoImage {
                baseindex: 0,
                tilesize: Some(512),
                level: vec![KrpanoLevel::Level(LevelAttributes {
                    tiledimagewidth: 3280,
                    tiledimageheight: 3280,
                    shape: vec![
                        Left(ShapeDesc {
                            url: TemplateString(vec![
                                str("https://example.com/"), y(4), str("/"),
                                x(5), str(".jpg")]),
                            multires: None,
                        })],
                })],
            })],
            ..Default::default()
        })
    }

    #[test]
    fn parse_xml_multires() {
        let parsed: KrpanoMetadata = serde_xml_rs::from_str(r#"
        <krpano>
        <image>
            <flat url="https://example.com/" multires="512,768x554,1664x1202,3200x2310,6400x4618,12800x9234"/>
        </image>
        </krpano>"#).unwrap();
        assert_eq!(parsed, KrpanoMetadata {
            children: vec![Image(KrpanoImage {
                baseindex: 1,
                tilesize: None,
                level: vec![KrpanoLevel::Flat(ShapeDesc {
                    url: TemplateString(vec![str("https://example.com/"), ]),
                    multires: Some("512,768x554,1664x1202,3200x2310,6400x4618,12800x9234".to_string()),
                })],
            })],
            ..Default::default()
        })
    }

    #[test]
    fn parse_xml_mobile() {
        // See https://github.com/lovasoa/dezoomify-rs/issues/58
        let parsed: KrpanoMetadata = serde_xml_rs::from_str(r#"
        <krpano>
        <image>
            <mobile>
                <cube url="test.jpg" />
            </mobile>
        </image>
        </krpano>"#).unwrap();
        assert_eq!(parsed, KrpanoMetadata {
            children: vec![Image(KrpanoImage {
                baseindex: 1,
                tilesize: None,
                level: vec![Mobile(vec![Cube(ShapeDesc {
                    url: TemplateString(vec![str("test.jpg")]),
                    multires: None,
                })])],
            })],
            ..Default::default()
        })
    }

    #[test]
    fn parse_xml_with_scene() {
        // See https://github.com/lovasoa/dezoomify-rs/issues/100#issuecomment-767048175
        let parsed: KrpanoMetadata = serde_xml_rs::from_str(r#"<krpano version="1.18">
        <scene name="scene_Color">
            <image type="CYLINDER" hfov="1.00" vfov="1.291661" voffset="0.00" multires="true" tilesize="512">
                <level tiledimagewidth="7424" tiledimageheight="9590">
                    <cylinder url="xxx/%0v/l5_%0v_%0h.jpg"/>
                </level>
            </image>
        </scene>
        </krpano>"#).unwrap();
        assert_eq!(parsed, KrpanoMetadata {
            children: vec![Scene(KrpanoMetadata {
                children: vec![Image(KrpanoImage {
                    baseindex: 1,
                    tilesize: Some(512),
                    level: vec![
                        KrpanoLevel::Level(LevelAttributes {
                            tiledimagewidth: 7424,
                            tiledimageheight: 9590,
                            shape: vec![
                                Cylinder(ShapeDesc {
                                    url: TemplateString(vec![
                                        str("xxx/"), y(2), str("/l5_"),
                                        y(2), str("_"), x(2), str(".jpg")
                                    ]),
                                    multires: None,
                                })
                            ],
                        })
                    ],
                })],
                name: "scene_Color".to_string()
            })],
            ..Default::default()
        })
    }

    #[test]
    fn parse_factum_arte() {
        // See https://github.com/lovasoa/dezoomify-rs/issues/100#issuecomment-767048175
        let f = std::fs::File::open("testdata/krpano/krpano_scenes.xml").unwrap();
        let parsed: KrpanoMetadata = serde_xml_rs::from_reader(f).unwrap();
        let infos: Vec<ImageInfo> = parsed.into_image_iter().collect();
        assert_eq!(infos.len(), 3);
        let names: Vec<String> = infos.iter().map(|i| String::from(i.name.as_ref())).collect();
        assert_eq!(names, ["scene_Color", "scene_3D", "scene_3Dcolor"])
    }

    #[test]
    fn parse_360cities() {
        // title: St George Hotel Dubai Tip Top English Disco by 360emirates
        let f = std::fs::File::open("testdata/krpano/krpano_360cities.xml").unwrap();
        let parsed: KrpanoMetadata = serde_xml_rs::from_reader(f).unwrap();
        let infos: Vec<ImageInfo> = parsed.into_image_iter().collect();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].image.level.len(), 4);
    }

    #[test]
    fn multires_parse() {
        let expected: Vec<Result<_, &'static str>> = vec![
            Ok((Vec2d { x: 6, y: 7 }, Vec2d { x: 3, y: 3 })),
            Ok((Vec2d { x: 8, y: 8 }, Vec2d { x: 3, y: 3 })),
            Ok((Vec2d { x: 9, y: 1 }, Vec2d { x: 4, y: 4 })),
        ];
        assert_eq!(expected, parse_multires("3,6x7,8x8,9x1x4").collect::<Vec<_>>())
    }

    #[test]
    fn test_templatestring() {
        assert_eq!(
            Ok(TemplateString(vec![
                x(3), str("%"), y(2), lvl(1)
            ])),
            "%00x%%%0y%l".parse()
        );
    }
}