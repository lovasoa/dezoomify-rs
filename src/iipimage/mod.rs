use crate::Vec2d;
use std::str::FromStr;
use custom_error::custom_error;
use std::sync::Arc;
use crate::dezoomer::{TilesRect, Dezoomer, DezoomerInput, ZoomLevels, DezoomerError, IntoZoomLevels, DezoomerInputWithContents};
use std::convert::TryFrom;
use std::iter::successors;
use std::fmt::Debug;
use regex::Regex;

/// A dezoomer for krpano images
/// See https://iipimage.sourceforge.io/documentation/protocol/
#[derive(Default)]
pub struct IIPImage;

const META_REQUEST_PARAMS: &str = "&OBJ=Max-size&OBJ=Tile-size&OBJ=Resolution-number";

impl Dezoomer for IIPImage {
    fn name(&self) -> &'static str { "IIPImage" }

    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        if data.uri.ends_with(META_REQUEST_PARAMS) {
            let DezoomerInputWithContents { uri, contents } = data.with_contents()?;
            let iter = iter_levels(uri, contents).map_err(DezoomerError::wrap)?;
            Ok(iter.into_zoom_levels())
        } else {
            let re = Regex::new("(?i)\\?FIF").unwrap();
            self.assert(re.is_match(&data.uri))?;
            let mut meta_uri: String = data.uri.chars().take_while(|&c| c != '&').collect();
            meta_uri += META_REQUEST_PARAMS;
            Err(DezoomerError::NeedsData { uri: meta_uri })
        }
    }
}

fn arcs<T, U: ?Sized>(v: T) -> impl Iterator<Item=Arc<U>>
    where Arc<U>: From<T> {
    successors(Some(Arc::from(v)), |x| Some(Arc::clone(x)))
}

fn iter_levels(uri: &str, contents: &[u8])
               -> Result<impl Iterator<Item=Level> + 'static, IIPError> {
    let base = String::from(uri.trim_end_matches(META_REQUEST_PARAMS));
    let meta = Metadata::try_from(contents)?;
    let levels =
        (0..meta.levels).zip(arcs(base)).zip(arcs(meta))
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IIPImage")
    }
}

impl TilesRect for Level {
    fn size(&self) -> Vec2d {
        let reverse_level = self.metadata.levels - self.level - 1;
        self.metadata.size / 2_u32.pow(reverse_level)
    }

    fn tile_size(&self) -> Vec2d { self.metadata.tile_size }

    fn tile_url(&self, Vec2d { x, y }: Vec2d) -> String {
        let Vec2d { x: width, .. } = self.size().ceil_div(self.tile_size());
        format!("{base}&JTL={level},{tile_index}",
                base = self.base,
                level = self.level,
                tile_index = y * width + x
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Metadata {
    size: Vec2d,
    tile_size: Vec2d,
    levels: u32,
}

impl FromStr for Metadata {
    type Err = IIPError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use IIPError::*;
        let mut size = Err(MissingKey { key: "Max-size" });
        let mut tile_size = Err(MissingKey { key: "Tile-size" });
        let mut levels = Err(MissingKey { key: "Resolution-number" });
        for line in s.lines() {
            let mut parts = line.split(':');
            let key: &str = parts.next().unwrap_or("").trim();
            let val: &str = parts.next().unwrap_or("").trim();
            let mut nums = val.split_ascii_whitespace().map(|s| s.parse::<u32>().ok());
            let n1 = nums.next().flatten();
            let n2 = nums.next().flatten();
            if key.eq_ignore_ascii_case("max-size") {
                if let (Some(x), Some(y)) = (n1, n2) {
                    size = Ok(Vec2d { x, y })
                }
            } else if key.eq_ignore_ascii_case("tile-size") {
                if let (Some(x), Some(y)) = (n1, n2) {
                    tile_size = Ok(Vec2d { x, y })
                }
            } else if key.eq_ignore_ascii_case("resolution-number") {
                if let Some(n) = n1 { levels = Ok(n) }
            }
        }
        Ok(Metadata { size: size?, tile_size: tile_size?, levels: levels? })
    }
}

impl TryFrom<&[u8]> for Metadata {
    type Error = IIPError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let s = std::str::from_utf8(value)?;
        Metadata::from_str(s)
    }
}

custom_error! {#[derive(PartialEq, Eq)] pub IIPError
    MissingKey{key: &'static str} = "missing key '{key}' in the IIPImage metadata file",
    Utf8{source: std::str::Utf8Error} = "Invalid IIPImage metadata file: {source}",
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dezoomer::PageContents;

    #[test]
    fn test_lowercase() {
        let uri = "https://publications-images.artic.edu/fcgi-bin/iipsrv.fcgi?fif=osci/Renoir_11/Color_Corrected/G39094sm2.ptif&jtl=4,11".to_string();
        let metadata_uri = "https://publications-images.artic.edu/fcgi-bin/iipsrv.fcgi?fif=osci/Renoir_11/Color_Corrected/G39094sm2.ptif&OBJ=Max-size&OBJ=Tile-size&OBJ=Resolution-number";
        let data = DezoomerInput { uri, contents: PageContents::Unknown };
        match IIPImage::default().zoom_levels(&data) {
            Err(DezoomerError::NeedsData { uri }) => assert_eq!(uri, metadata_uri),
            _ => panic!("Unexpected result")
        }
    }

    #[test]
    fn test_parse_metadata() {
        let contents = &b"Max-size:512 512\nTile-size:256 256\nResolution-number:2"[..];
        let base: Arc<str> = Arc::from("http://test.com/");
        let levels: Vec<Level> = iter_levels(&base, contents).unwrap().collect();
        assert_eq!(&levels, &[
            Level {
                metadata: Arc::from(Metadata {
                    size: Vec2d { x: 512, y: 512 },
                    tile_size: Vec2d { x: 256, y: 256 },
                    levels: 2,
                }),
                base: base.clone(),
                level: 0,
            },
            Level {
                metadata: Arc::from(Metadata {
                    size: Vec2d { x: 512, y: 512 },
                    tile_size: Vec2d { x: 256, y: 256 },
                    levels: 2,
                }),
                base,
                level: 1,
            }
        ]);
        assert_eq!(levels[0].tile_url(Vec2d { x: 0, y: 0 }), "http://test.com/&JTL=0,0");
        assert_eq!(levels[1].tile_url(Vec2d { x: 0, y: 1 }), "http://test.com/&JTL=1,2");
    }

    #[test]
    fn test_zoom_levels() {
        let source = "
        Max-size:23235 23968
        Tile-size:256 256
        Resolution-number:9
    ";
        assert_eq!(source.parse(), Ok(Metadata {
            size: Vec2d { x: 23235, y: 23968 },
            tile_size: Vec2d { x: 256, y: 256 },
            levels: 9,
        }))
    }
}