use std::borrow::Cow;
use std::collections::HashSet;

use lazy_static::lazy_static;
use log::info;
use log::warn;
use serde::{Deserialize, Serialize};

use crate::Vec2d;

#[derive(Default, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImageInfo {
    #[serde(rename = "@context", skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    #[serde(alias = "@type")]
    pub iiif_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
    #[serde(rename = "@id", skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub width: u32,
    pub height: u32,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualities: Option<Vec<String>>,

    #[serde(alias = "preferredFormats", skip_serializing_if = "Option::is_none")]
    pub formats: Option<Vec<String>>,

    // Used in IIIF version 2 :
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tiles: Option<Vec<TileInfo>>,

    // Used in IIIF version 1 :
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_factors: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile_height: Option<u32>,
}

// Image qualities, from least favorite to favorite
static QUALITY_ORDER: [&str; 5] = ["bitonal", "gray", "color", "native", "default"];

// Image formats, from least favorite to favorite
// webp is the least favorite because of this bug: https://github.com/image-rs/image/issues/939
static FORMAT_ORDER: [&str; 7] = ["webp", "gif", "bmp", "tif", "png", "jpg", "jpeg"];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TileSizeFormat { WidthHeight, Width }

impl ImageInfo {
    pub fn size(&self) -> Vec2d {
        Vec2d {
            x: self.width,
            y: self.height,
        }
    }

    fn profile_info(&self) -> Cow<ProfileInfo> {
        self.profile.as_ref().map(|p| p.profile_info()).unwrap_or_default()
    }

    pub fn best_quality(&self) -> String {
        let pinfo = self.profile_info();
        self.qualities.iter().flat_map(|v| v.iter())
            .chain(pinfo.qualities.iter().flat_map(|x| x.iter()))
            .max_by_key(|&s| QUALITY_ORDER.iter().position(|&x| x == s))
            .cloned()
            .unwrap_or_else(|| {
                info!("No image quality specified. Using 'default'.");
                "default".into()
            })
    }

    pub fn best_format(&self) -> String {
        let pinfo = self.profile_info();
        self.formats.iter().flat_map(|v| v.iter())
            .chain(pinfo.formats.iter().flat_map(|x| x.iter()))
            .max_by_key(|&s| FORMAT_ORDER.iter().position(|&x| x == s))
            .cloned()
            .unwrap_or_else(|| {
                info!("No image format specified. Using 'jpg'.");
                "jpg".into()
            })
    }

    pub fn preferred_size_format(&self) -> TileSizeFormat {
        let pinfo = self.profile_info();
        let s: HashSet<&str> = pinfo.supports.iter()
            .flat_map(|x| x.iter())
            .map(|s| s.as_str())
            .collect();
        if s.contains("sizeByW") && !s.contains("sizeByWh") {
            TileSizeFormat::Width
        } else {
            TileSizeFormat::WidthHeight
        }
    }

    pub fn tiles(&self) -> Vec<TileInfo> {
        self.tiles.as_ref()
            .and_then(|v|
                if v.is_empty() {
                    None
                } else {
                    Some(v.to_vec())
                })
            .unwrap_or_else(|| {
                let mut info = TileInfo::default();
                if let Some(width) = self.tile_width {
                    info.width = width
                }
                if let Some(height) = self.tile_height {
                    info.height = Some(height)
                }
                if let Some(scale_factors) = &self.scale_factors {
                    info.scale_factors = scale_factors.clone()
                }
                vec![info]
            })
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct TileInfo {
    pub width: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(rename = "scaleFactors")]
    pub scale_factors: Vec<u32>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(untagged)]
pub enum Profile {
    Reference(String),
    Info(ProfileInfo),
    Multiple(Option<Vec<Profile>>),
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Default)]
pub struct ProfileInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "extraFormats")]
    formats: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "extraQualities")]
    qualities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "extraFeatures")]
    supports: Option<Vec<String>>,
}

lazy_static! {
    static ref PROFILE_REFERENCES: std::collections::HashMap<String, ProfileInfo> = {
        let json_str = include_str!("./levels.json");
        serde_json::from_str(json_str).unwrap()
    };
}

impl Profile {
    fn profile_info(&self) -> Cow<ProfileInfo> {
        match self {
            Profile::Reference(s) => {
                PROFILE_REFERENCES.get(s)
                    .map(|x| Cow::Borrowed(x))
                    .unwrap_or_else(|| {
                        warn!("Unknown IIIF profile reference: {}", s);
                        Cow::Owned(ProfileInfo::default())
                    })
            },
            Profile::Info(info) => { Cow::Borrowed(info) },
            Profile::Multiple(profiles) => {
                let mut formats = vec![];
                let mut qualities = vec![];
                let mut supports = vec![];
                for profile in profiles.iter().flat_map(|x| x.iter()) {
                    let p = profile.profile_info();
                    if let Some(x) = &p.formats { formats.extend_from_slice(x) }
                    if let Some(x) = &p.qualities { qualities.extend_from_slice(x) }
                    if let Some(x) = &p.supports { supports.extend_from_slice(x) }
                }
                Cow::Owned(ProfileInfo {
                    formats: Some(formats),
                    qualities: Some(qualities),
                    supports: Some(supports),
                })
            },
        }
    }
}


impl Default for TileInfo {
    fn default() -> Self {
        TileInfo {
            width: 512,
            height: None,
            scale_factors: vec![1],
        }
    }
}

#[test]
fn test_deserialisation() {
    let _: ImageInfo = serde_json::from_str(
        r#"{
      "@context" : "http://iiif.io/api/image/2/context.json",
      "@id" : "http://www.example.org/image-service/abcd1234/1E34750D-38DB-4825-A38A-B60A345E591C",
      "protocol" : "http://iiif.io/api/image",
      "width" : 6000,
      "height" : 4000,
      "sizes" : [
        {"width" : 150, "height" : 100},
        {"width" : 600, "height" : 400},
        {"width" : 3000, "height": 2000}
      ],
      "tiles": [
        {"width" : 512, "scaleFactors" : [1,2,4,8,16]}
      ],
      "profile" : [ "http://iiif.io/api/image/2/level2.json" ]
    }"#,
    )
    .unwrap();
}

#[test]
fn test_profile_info() {
    let profiles = Profile::Multiple(Some(vec![
        Profile::Reference("http://iiif.io/api/image/2/level0.json".into()),
        Profile::Info(ProfileInfo {
            formats: None,
            qualities: None,
            supports: Some(vec!["sizeByWh".into()]),
        })
    ]));
    use std::ops::Deref;
    assert_eq!(profiles.profile_info().deref(), &ProfileInfo {
        formats: Some(vec!["jpg".into()]), // from level0
        qualities: Some(vec!["default".into()]), // from level0
        supports: Some(vec![
            "sizeByWhListed".into(), // from level0
            "sizeByWh".into(), // from the second profile
        ]),
    })
}

#[test]
fn test_best_quality() {
    let pairs = vec![
        (None, "default"),
        (Some(vec![]), "default"),
        (Some(vec!["color".into()]), "color"),
        (Some(vec!["grey".into()]), "grey"),
        (Some(vec!["zorglub".into()]), "zorglub"),
        (Some(vec!["zorglub".into(), "color".into()]), "color"),
        (Some(vec!["bitonal".into(), "gray".into()]), "gray"),
        (Some(vec!["bitonal".into(), "gray".into(), "color".into()]), "color"),
        (Some(vec!["default".into(), "bitonal".into(), "gray".into(), "color".into()]), "default"),
    ];
    for (qualities, expected_best_quality) in pairs.into_iter() {
        let info = ImageInfo { qualities, ..ImageInfo::default() };
        assert_eq!(info.best_quality(), expected_best_quality);
    }
}