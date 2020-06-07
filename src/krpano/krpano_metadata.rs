use std::str::FromStr;
use std::sync::Arc;

use serde::{de, Deserialize, Deserializer};

#[derive(Debug, Deserialize, PartialEq)]
pub struct KrpanoMetadata {
    pub image: Vec<KrpanoImage>,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct KrpanoImage {
    pub tilesize: u32,
    #[serde(default = "default_base_index")]
    pub baseindex: u32,
    pub level: Vec<KrpanoLevel>,
}

fn default_base_index() -> u32 { 1 }

#[derive(Debug, Deserialize, PartialEq)]
pub struct KrpanoLevel {
    pub tiledimagewidth: u32,
    pub tiledimageheight: u32,
    #[serde(rename = "$value")]
    pub shape: Vec<Shape>,
}


#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    Cube { url: TemplateString<TemplateVariable> },
    Cylinder { url: TemplateString<TemplateVariable> },
    Flat { url: TemplateString<TemplateVariable> },
    Left { url: TemplateString<TemplateVariable> },
    Right { url: TemplateString<TemplateVariable> },
    Front { url: TemplateString<TemplateVariable> },
    Back { url: TemplateString<TemplateVariable> },
    Up { url: TemplateString<TemplateVariable> },
    Down { url: TemplateString<TemplateVariable> },
}

impl Shape {
    pub fn name_and_url(self) -> (&'static str, TemplateString<TemplateVariable>) {
        match self {
            Self::Cube { url } => ("Cube", url),
            Self::Cylinder { url } => ("Cylinder", url),
            Self::Flat { url } => ("Flat", url),
            Self::Left { url } => ("Left", url),
            Self::Right { url } => ("Right", url),
            Self::Front { url } => ("Front", url),
            Self::Back { url } => ("Back", url),
            Self::Up { url } => ("Up", url),
            Self::Down { url } => ("Down", url),
        }
    }
}

#[derive(Debug, PartialEq)]
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
        let mut chars = input.chars();
        let mut parts = vec![];
        loop {
            let literal: String = chars.take_while_ref(|&c| c != '%').collect();
            parts.push(TemplateStringPart::Literal(Arc::new(literal)));
            if chars.next().is_none() { break; }
            let padding = chars.take_while_ref(|&c| c == '0').count() as u32;
            let variable = match chars.next() {
                Some('h') | Some('x') | Some('u') | Some('c') => TemplateVariable::X,
                Some('v') | Some('y') | Some('r') => TemplateVariable::Y,
                Some('s') => TemplateVariable::Side,
                Some(x) => return Err(format!("Unknown template variable '{}' in '{}'", x, input)),
                None => return Err(format!("Invalid templating syntax in '{}'", input))
            };
            parts.push(TemplateStringPart::Variable { padding, variable })
        }
        Ok(TemplateString(parts))
    }
}

impl TemplateString<TemplateVariable> {
    pub fn all_sides(self) -> impl Iterator<Item=(&'static str, TemplateString<XY>)> + 'static {
        let has_side = self.0.iter().any(|x| match x {
            TemplateStringPart::Variable { variable, .. } => *variable == TemplateVariable::Side,
            _ => false
        });
        let sides = if has_side { &["forward", "back", "left", "right", "up", "down"][..] } else { &[""] };
        sides.iter().map(move |&side| (side, TemplateString(
            self.0.iter().map(|part| part.with_side(side)).collect()
        )))
    }
}


#[derive(Debug, PartialEq, Clone)]
pub enum TemplateStringPart<T> {
    Literal(Arc<String>),
    Variable { padding: u32, variable: T },
}

impl TemplateStringPart<TemplateVariable> {
    fn with_side(&self, side: &'static str) -> TemplateStringPart<XY> {
        match self {
            TemplateStringPart::Literal(s) => TemplateStringPart::Literal(Arc::clone(s)),
            TemplateStringPart::Variable { padding, variable } => {
                let padding = *padding;
                match variable {
                    TemplateVariable::X => TemplateStringPart::Variable { padding, variable: XY::X },
                    TemplateVariable::Y => TemplateStringPart::Variable { padding, variable: XY::Y },
                    TemplateVariable::Side => TemplateStringPart::Literal(Arc::new(side[..1].to_string())),
                }
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum TemplateVariable { X, Y, Side }

#[derive(Debug, PartialEq)]
pub enum XY { X, Y }

#[cfg(test)]
mod test {
    use crate::krpano::krpano_metadata::Shape::Left;
    use crate::krpano::krpano_metadata::TemplateStringPart::{Literal, Variable};
    use crate::krpano::krpano_metadata::TemplateVariable::{X, Y};

    use super::*;

    fn str(s: &str) -> TemplateStringPart<TemplateVariable> { Literal(Arc::new(s.to_string())) }

    fn x(padding: u32) -> TemplateStringPart<TemplateVariable> { Variable { padding, variable: X } }

    fn y(padding: u32) -> TemplateStringPart<TemplateVariable> { Variable { padding, variable: Y } }

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
        assert_eq!(parsed, KrpanoMetadata {
            image: vec![
                KrpanoImage {
                    baseindex: 1,
                    tilesize: 512,
                    level: vec![
                        KrpanoLevel {
                            tiledimagewidth: 31646,
                            tiledimageheight: 38234,
                            shape: vec![Shape::Cylinder {
                                url: TemplateString(vec![
                                    str("monomane.tiles/l7/"), y(0), str("/l7_"),
                                    y(0), str("_"), x(0), str(".jpg"),
                                ])
                            }],
                        },
                    ],
                }
            ]
        })
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
            image: vec![KrpanoImage {
                baseindex: 0,
                tilesize: 512,
                level: vec![KrpanoLevel {
                    tiledimagewidth: 3280,
                    tiledimageheight: 3280,
                    shape: vec![
                        Left {
                            url: TemplateString(vec![
                                str("https://example.com/"), y(3), str("/"),
                                x(4), str(".jpg")])
                        }],
                }],
            }]
        })
    }
}