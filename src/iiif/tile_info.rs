use std::borrow::Cow;
use std::collections::HashSet;

use lazy_static::lazy_static;
use log::info;
use log::warn;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::Vec2d;

#[derive(Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
static FORMAT_ORDER: [&str; 7] = ["webp", "gif", "bmp", "tif", "jpg", "jpeg", "png"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
        let profile_info = self.profile_info();
        let mut tiles = self.tiles.as_ref()
            .map(|v| {
                v.iter().flat_map(|info|
                    if profile_info.tile_size_fits(info.size()) {
                        Some(info.clone())
                    } else { None }
                ).collect()
            }).unwrap_or_else(Vec::new);
        // If no preset tile size covers the full-resolution image, add a new one
        if !tiles.iter().any(|t| t.scale_factors.contains(&1)) {
            let mut info = TileInfo::default();
            if let Some(width) = self.tile_width {
                info.width = width
            }
            if let Some(height) = self.tile_height {
                info.height = Some(height)
            }
            let cropped_size = profile_info.crop_tile_size(info.size());
            info.width = cropped_size.x;
            info.height = Some(cropped_size.y);
            if let Some(scale_factors) = &self.scale_factors {
                info.scale_factors = scale_factors.clone()
            }
            tiles.push(info)
        }
        tiles
    }

    /// Because our parser is so tolerant, we need to evaluate the probability
    /// that this is not in fact a valid IIIF image
    pub fn has_distinctive_iiif_properties(&self) -> bool {
        self.id.is_some() || self.protocol.is_some() || self.context.is_some() ||
            self.tiles.is_some() || self.formats.is_some() ||
            self.iiif_type.as_ref().filter(
                |&s| s == "iiif:ImageProfile" || s == "ImageService3"
            ).is_some()
    }

    /// Some info.json files contain a an invalid value for "@id",
    /// such as "localhost" or "example.com"
    pub fn remove_test_id(&mut self) {
        if let Some(id) = &self.id {
            if Regex::new(r"^https?://((www\.)?example\.|localhost)").unwrap().is_match(id) {
                info!("Removing probably invalid IIIF id '{}'", id);
                self.id = None;
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct TileInfo {
    pub width: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(rename = "scaleFactors")]
    pub scale_factors: Vec<u32>,
}

impl TileInfo {
    pub fn size(&self) -> Vec2d {
        Vec2d { x: self.width, y: self.height.unwrap_or(self.width) }
    }
}


#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(untagged)]
pub enum Profile {
    Reference(String),
    Info(ProfileInfo),
    Multiple(Option<Vec<Profile>>),
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, Default)]
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
    #[serde(rename = "maxWidth")]
    max_width: Option<u32>,
    #[serde(rename = "maxHeight")]
    max_height: Option<u32>,
    #[serde(rename = "maxArea")]
    max_area: Option<u64>,
}

impl ProfileInfo {
    /// Takes a proposed tile_size as input and returns a potentially smaller tile size
    /// that fits the tile size limits defined in the profile information
    fn crop_tile_size(&self, mut size: Vec2d) -> Vec2d {
        if let Some(max_width) = self.max_width {
            size.x = size.x.min(max_width);
            let max_height = self.max_height.unwrap_or(max_width);
            size.y = size.y.min(max_height);
        }
        if let Some(max_area) = self.max_area {
            if size.area() > max_area {
                let sqrt = ((max_area as f64).sqrt()) as u32;
                size.y = sqrt.min(size.y);
                size.x = sqrt.min(size.x);
            }
        }
        size
    }
    /// Whether the given tile size fits the maximum size constraints in the profile
    fn tile_size_fits(&self, size: Vec2d) -> bool {
        self.crop_tile_size(size) == size
    }
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
                    .map(Cow::Borrowed)
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
                let mut max_width = None;
                let mut max_height = None;
                let mut max_area = None;
                fn update_max<T: Ord + Copy>(target: &mut Option<T>, new: Option<T>) {
                    if let Some(new) = new {
                        *target = Some(
                            if let Some(old) = target { new.min(*old) } else { new }
                        )
                    }
                }
                for profile in profiles.iter().flat_map(|x| x.iter()) {
                    let p = profile.profile_info();
                    if let Some(x) = &p.formats { formats.extend_from_slice(x) }
                    if let Some(x) = &p.qualities { qualities.extend_from_slice(x) }
                    if let Some(x) = &p.supports { supports.extend_from_slice(x) }
                    update_max(&mut max_width, p.max_width);
                    update_max(&mut max_height, p.max_height);
                    update_max(&mut max_area, p.max_area);
                }
                Cow::Owned(ProfileInfo {
                    formats: Some(formats),
                    qualities: Some(qualities),
                    supports: Some(supports),
                    max_width,
                    max_height,
                    max_area,
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
            supports: Some(vec!["sizeByWh".into()]),
            max_width: Some(56),
            ..Default::default()
        }),
        Profile::Info(ProfileInfo {
            max_width: Some(78),
            max_height: Some(94),
            ..Default::default()
        })
    ]));
    assert_eq!(*profiles.profile_info(), ProfileInfo {
        formats: Some(vec!["jpg".into()]), // from level0
        qualities: Some(vec!["default".into()]), // from level0
        supports: Some(vec![
            "sizeByWhListed".into(), // from level0
            "sizeByWh".into(), // from the second profile
        ]),
        max_width: Some(56),
        max_height: Some(94),
        ..Default::default()
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
