use crate::Vec2d;
use std::str::FromStr;
use custom_error::custom_error;
use std::sync::Arc;
use crate::dezoomer::{TilesRect, Dezoomer, DezoomerInput, ZoomLevels, DezoomerError, IntoZoomLevels, DezoomerInputWithContents};
use std::convert::TryFrom;
use std::iter::successors;
use std::fmt::Debug;
use serde::export::Formatter;

/// A dezoomer for NYPL images
#[derive(Default)]
pub struct NYPLImage;

const NYPL_IMAGE_VIEW_PREFIX: &str = "https://digitalcollections.nypl.org/items/";
const NYPL_META_PREFIX: &str = "https://access.nypl.org/image.php/";
const NYPL_META_POSTFIX: &str = "/tiles/config.js";

fn _get_image_url_from_meta_url(meta_url: &str) -> String {
    meta_url.replace(NYPL_META_PREFIX, "")
        .replace(NYPL_META_POSTFIX, "")
}

macro_rules! nypl_meta_format_tpl {
    () => {
        "https://access.nypl.org/image.php/{}/tiles/config.js"
    }
}

impl Dezoomer for NYPLImage {
    fn name(&self) -> &'static str { "NYPLImage" }
    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        if data.uri.starts_with(NYPL_IMAGE_VIEW_PREFIX) {
            self.assert(data.uri.contains(NYPL_IMAGE_VIEW_PREFIX))?;
            let image_id = data.uri.replace(NYPL_IMAGE_VIEW_PREFIX, "");
            let meta_uri = format!(nypl_meta_format_tpl!(), image_id);
            Err(DezoomerError::NeedsData { uri: meta_uri })
        } else {
            self.assert(data.uri.contains(NYPL_META_PREFIX))?;
            let DezoomerInputWithContents { uri, contents } = data.with_contents()?;
            let iter = iter_levels(uri, contents).map_err(DezoomerError::wrap)?;
            Ok(iter.into_zoom_levels())
        }
    }
}

fn arcs<T>(v: T) -> impl Iterator<Item=Arc<T>> {
    successors(Some(Arc::new(v)), |x| Some(Arc::clone(x)))
}

fn iter_levels(uri: &str, contents: &[u8])
               -> Result<impl Iterator<Item=Level> + 'static, NYPLError> {
    let base = _get_image_url_from_meta_url(uri);
    let meta = Metadata::try_from(contents)?;
    let levels =
        (0..meta.levels).zip(arcs(base)).zip(arcs(meta))
            .map(|((level, base), metadata)|
                Level { metadata, base, level });
    Ok(levels)
}

#[derive(PartialEq)]
struct Level {
    metadata: Arc<Metadata>,
    base: Arc<String>,
    level: u32,
}

impl Debug for Level {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "NYPLImage")
    }
}

impl TilesRect for Level {
    fn size(&self) -> Vec2d {
        let reverse_level = self.metadata.levels - self.level - 1;
        self.metadata.size / 2_u32.pow(reverse_level)
    }

    fn tile_size(&self) -> Vec2d { self.metadata.tile_size }

    fn tile_url(&self, Vec2d { x, y }: Vec2d) -> String {
        let Vec2d { x: _width, .. } = self.size().ceil_div(self.tile_size());
        format!("https://access.nypl.org/image.php/{id}/tiles/0/12/{x}_{y}.png",
                id = self.base,
                x = x,
                y = y,
        )
    }
}

#[derive(Debug, PartialEq)]
pub struct Metadata {
    size: Vec2d,
    tile_size: Vec2d,
    levels: u32,
}

impl FromStr for Metadata {
    type Err = NYPLError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use NYPLError::*;
        let _parsed = json::parse(s);
        if _parsed.is_err(){
            return Err(JsonError{resp: s.to_string()});
        }
        let parsed = _parsed.unwrap();
        let meta = parsed["configs"]["0"].to_owned();
        let width = meta["size"]["width"].as_str().unwrap()
            .parse::<u32>().unwrap();
        let height = meta["size"]["height"].as_str().unwrap()
            .parse::<u32>().unwrap();
        let _tile_width = meta["tilesize"].as_str().unwrap()
            .parse::<u32>().unwrap();
        let size = Vec2d { x: width, y: height };
        let tile_size = Vec2d{x: _tile_width, y: _tile_width};
        let levels= 1;
        Ok(Metadata { size, tile_size, levels })
    }
}

impl TryFrom<&[u8]> for Metadata {
    type Error = NYPLError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let s = std::str::from_utf8(value)?;
        Metadata::from_str(s)
    }
}

custom_error! {#[derive(PartialEq)] pub NYPLError
    JsonError{resp: String} = "Failed to parse NYPL Image meta as json, \
        got content(blank shows the site has no zoom function for this one):\n {resp}",
    Utf8{source: std::str::Utf8Error} = "Invalid NYPLImage metadata file: {}",
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
        let levels: Vec<Level> = iter_levels(&base, contents).unwrap().collect();
        assert_eq!(&levels, &[
            Level {
                metadata: Arc::from(Metadata {
                    size: Vec2d { x: 2422, y: 3000 },
                    tile_size: Vec2d { x: 256, y: 256 },
                    levels: 1,
                }),
                base: base.clone(),
                level: 0,
            },
        ]);
        let expected_url = "https://access.nypl.org/image.php/\
            a28d6e6b-b317-f008-e040-e00a1806635d\
            /tiles/0/12/0_0.png";
        assert_eq!(levels[0].tile_url(Vec2d { x: 0, y: 0 }), expected_url);
    }
}