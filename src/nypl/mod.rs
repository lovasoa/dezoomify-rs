use std::sync::Arc;
use std::iter::successors;
use std::fmt::{Debug, Formatter};
use std::collections::HashMap;

use custom_error::custom_error;
use regex::Regex;
use serde::Deserialize;

use crate::dezoomer::{TilesRect, Dezoomer, DezoomerInput, ZoomLevels, DezoomerError, IntoZoomLevels, DezoomerInputWithContents, TileReference};
use crate::json_utils::number_or_string;
use crate::Vec2d;

/// A dezoomer for NYPL images
#[derive(Default)]
pub struct NYPLImage;

const NYPL_IMAGE_VIEW_PREFIX: &str = "https://digitalcollections.nypl.org/items/";
const NYPL_META_PREFIX: &str = "https://access.nypl.org/image.php/";
const NYPL_META_POSTFIX: &str = "/tiles/config.js";

fn get_image_id_from_meta_url(meta_url: &str) -> String {
    meta_url.replace(NYPL_META_PREFIX, "")
        .replace(NYPL_META_POSTFIX, "")
}

fn parse_image_id(image_view_url: &str) -> Option<String> {
    Regex::new(r"https://digitalcollections.nypl.org/items/([a-f0-9\-]+)").unwrap()
        .captures(image_view_url)
        .and_then(|cap| cap.get(1))
        .map(|m| m.as_str().to_string())
}

impl Dezoomer for NYPLImage {
    fn name(&self) -> &'static str { "nypl" }
    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        if data.uri.starts_with(NYPL_IMAGE_VIEW_PREFIX) {
            let image_view_url = data.uri.as_str();
            let image_id = parse_image_id(image_view_url).ok_or_else(||
                DezoomerError::wrap(NYPLError::NoIdInUrl { url: image_view_url.to_string() })
            )?;
            let meta_uri = format!("{}{}{}", NYPL_META_PREFIX, image_id, NYPL_META_POSTFIX);
            Err(DezoomerError::NeedsData { uri: meta_uri })
        } else {
            self.assert(data.uri.contains(NYPL_META_PREFIX))?;
            let DezoomerInputWithContents { uri, contents } = data.with_contents()?;
            let iter = iter_levels(uri, contents).map_err(DezoomerError::wrap)?;
            Ok(iter.into_zoom_levels())
        }
    }
}

fn arcs<T, U: ?Sized>(v: T) -> impl Iterator<Item=Arc<U>>
    where Arc<U>: From<T> {
    successors(Some(Arc::from(v)), |x| Some(Arc::clone(x)))
}

fn iter_levels(uri: &str, contents: &[u8])
               -> Result<impl Iterator<Item=Level> + 'static, NYPLError> {
    if contents.is_empty() {
        return Err(NYPLError::NoMetadata);
    }
    let base = get_image_id_from_meta_url(uri);
    let mut meta_map: MetadataRoot = serde_json::from_slice(contents)?;
    let (_, meta) = meta_map.configs.drain()
        .find(|(k, _v)| k == "0")
        .ok_or(NYPLError::NoMetadata)?;

    let level_count: u32 = meta.level_count();
    let levels =
        (0..=level_count).zip(arcs(base)).zip(arcs(meta))
            .map(|((level, base), metadata)|
                Level { metadata, base, level });
    Ok(levels)
}

#[derive(PartialEq, Eq)]
struct Level {
    metadata: Arc<Metadata>,
    base: Arc<str>,
    level: u32,
}

impl Debug for Level {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "NYPL Image")
    }
}

impl TilesRect for Level {
    fn size(&self) -> Vec2d {
        let reverse_level = self.metadata.level_count() - self.level;
        Vec2d::from(self.metadata.size) / 2_u32.pow(reverse_level)
    }

    fn tile_size(&self) -> Vec2d { Vec2d::square(self.metadata.tile_size) }

    fn tile_url(&self, Vec2d { x, y }: Vec2d) -> String {
        format!("https://access.nypl.org/image.php/{id}/tiles/0/{level}/{x}_{y}.{format}",
                id = self.base,
                level = self.level,
                x = x,
                y = y,
                format = self.metadata.format,
        )
    }

    fn tile_ref(&self, pos: Vec2d) -> TileReference {
        let delta = Vec2d {
            x: if pos.x == 0 { 0 } else { self.metadata.overlap },
            y: if pos.y == 0 { 0 } else { self.metadata.overlap },
        };
        TileReference {
            url: self.tile_url(pos),
            position: self.tile_size() * pos - delta,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct MetadataRoot {
    configs: HashMap<String, Metadata>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct Metadata {
    size: MetadataSize,
    #[serde(alias = "tilesize", deserialize_with = "number_or_string")]
    tile_size: u32,
    format: String,
    #[serde(default = "Default::default", deserialize_with = "number_or_string")]
    overlap: u32,
}

impl Metadata {
    fn level_count(&self) -> u32 {
        let max_dim: u32 = self.size.width.max(self.size.height);
        32 - max_dim.leading_zeros()
    }
}

impl From<MetadataSize> for Vec2d {
    fn from(s: MetadataSize) -> Self {
        Vec2d { x: s.width, y: s.height }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Deserialize)]
struct MetadataSize {
    #[serde(deserialize_with = "number_or_string")]
    width: u32,
    #[serde(deserialize_with = "number_or_string")]
    height: u32,
}

custom_error! {pub NYPLError
    JsonError{resp: String} = "Failed to parse NYPL Image meta as json, \
        got content(blank shows the site has no zoom function for this one):\n {resp}",
    Utf8{source: std::str::Utf8Error} = "Invalid NYPL metadata file: {source}",
    NoIdInUrl{url: String} = "Unable to extract an image id from {url:?}",
    BadMetadata{source: serde_json::Error} = "Invalid nypl metadata: {source}",
    NoMetadata = "No metadata found. This image is probably not tiled, \
    and you can download it directly by right-clicking on it from \
    your browser without any external tool.",
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_metadata() {
        let contents = r#"
        {
          "configs":{
            "0":{
              "size":{
                "width":"2422",
                "height":"3000"
              },
              "tilesize":"256",
              "overlap":"2",
              "format":"png"
            },
            "90":{
              "size":{
                "width":"3000",
                "height":"2422"
              },
              "tilesize":"256",
              "overlap":"2",
              "format":"png"
            },
            "180":{
              "size":{
                "width":"2422",
                "height":"3000"
              },
              "tilesize":"256",
              "overlap":"2",
              "format":"png"
            },
            "270":{
              "size":{
                "width":"3000",
                "height":"2422"
              },
              "tilesize":"256",
              "overlap":"2",
              "format":"png"
            }
          }
        }
        "#.as_bytes();
        let base: Arc<String> = Arc::new("a28d6e6b-b317-f008-e040-e00a1806635d".into());
        let level: Level = iter_levels(&base, contents).unwrap().last().unwrap();
        assert_eq!(level.metadata, Arc::new(Metadata {
            size: MetadataSize { width: 2422, height: 3000 },
            tile_size: 256,
            format: "png".to_string(),
            overlap: 2,
        }));
        let expected_url = "https://access.nypl.org/image.php/\
            a28d6e6b-b317-f008-e040-e00a1806635d\
            /tiles/0/12/0_0.png";
        assert_eq!(level.tile_url(Vec2d { x: 0, y: 0 }), expected_url);
        assert_eq!(
            parse_image_id(
                "https://digitalcollections.nypl.org/items/a14f3200-fac1-012f-f7a4-58d385a7bbd0#item-data"
            ).unwrap(),
            "a14f3200-fac1-012f-f7a4-58d385a7bbd0",
        )
    }
}