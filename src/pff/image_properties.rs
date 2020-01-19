use std::num::ParseIntError;
use std::str::FromStr;

use serde::{Deserialize, Deserializer};
use serde::export::fmt::Debug;

use custom_error::custom_error;

use crate::dezoomer::Vec2d;

#[derive(Debug, Deserialize, PartialEq)]
struct Reply<T: FromStr> where <T as FromStr>::Err: ToString {
    #[serde(deserialize_with = "deserialize_from_string")]
    reply_data: T,
}

fn deserialize_from_string<'d, D: Deserializer<'d>, T: FromStr>(de: D) -> Result<T, D::Error>
    where <T as FromStr>::Err: ToString {
    let as_str = String::deserialize(de)?;
    FromStr::from_str(&as_str)
        .map_err(|e: <T as FromStr>::Err| serde::de::Error::custom(e.to_string()))
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct PffHeader {
    #[serde(rename = "WIDTH", default)]
    pub width: u32,
    #[serde(rename = "HEIGHT", default)]
    pub height: u32,
    #[serde(rename = "TILESIZE", default)]
    pub tile_size: u32,
    #[serde(rename = "NUMTILES", default)]
    pub num_tiles: u32,
    #[serde(rename = "HEADERSIZE", default)]
    pub header_size: u32,
    #[serde(rename = "VERSION", default)]
    pub version: u32,
}


impl FromStr for PffHeader {
    type Err = serde_xml_rs::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_xml_rs::from_str(s)
    }
}


impl PffHeader {
    fn size(&self) -> Vec2d {
        Vec2d {
            x: self.width,
            y: self.height,
        }
    }
    fn tile_size(&self) -> Vec2d {
        Vec2d {
            x: self.tile_size,
            y: self.tile_size,
        }
    }

    /// Returns the index in the file of the first and last bytes of the tiles data
    fn index_bytes(&self) -> (u32, u32) {
        let begin = 0x424 + self.header_size;
        let end = begin + 8 * self.num_tiles;
        (begin, end)
    }

    pub fn levels(&self) -> impl Iterator<Item=ZoomLevelInfo> {
        let mut remaining_tiles = i64::from(self.num_tiles);
        let mut size = self.size();
        let tile_size = self.tile_size();
        std::iter::from_fn(move || {
            if remaining_tiles <= 0 {
                None
            } else {
                let Vec2d {
                    x: tiles_x,
                    y: tiles_y,
                } = size.ceil_div(tile_size);
                remaining_tiles -= i64::from(tiles_x) * i64::from(tiles_y);
                let tiles_before = remaining_tiles as u32;
                let lvl = ZoomLevelInfo {
                    tiles_before,
                    tile_size,
                    size,
                };
                size = size.ceil_div(Vec2d { x: 2, y: 2 });
                Some(lvl)
            }
        })
    }
}

pub struct ZoomLevelInfo {
    pub size: Vec2d,
    pub tile_size: Vec2d,
    pub tiles_before: u32,
}

impl ZoomLevelInfo {
    pub fn tile_group(&self, pos: Vec2d) -> u32 {
        let num_tiles_x = (self.size.ceil_div(self.tile_size)).x;
        (self.tiles_before + pos.x + pos.y * num_tiles_x) / 256
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct TileIndices {
    indices: Vec<u64>
}

custom_error! {#[derive(PartialEq, Eq)] pub ParseTileIndicesError
    TooShort = "Missing a part of tile indices string",
    BadNum{source: ParseIntError} = "Invalid tile index: {}",
}

impl FromStr for TileIndices {
    type Err = ParseTileIndicesError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(',');
        let first: u64 = parts.next().ok_or(ParseTileIndicesError::TooShort)?.parse()?;
        let mut rest_str: &str = parts.next().ok_or(ParseTileIndicesError::TooShort)?;
        let other_nums = rest_str.split_ascii_whitespace();
        let indices = other_nums
            .map(|num_str| num_str.parse::<u64>().map(|x| first + x))
            .collect::<Result<Vec<u64>, _>>()?;
        Ok(TileIndices { indices })
    }
}


#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_tile_indices() {
        assert_eq!("10, 1 2 3".parse(), Ok(TileIndices { indices: vec![11, 12, 13] }));
        assert_eq!("10,        0       20".parse(), Ok(TileIndices { indices: vec![10, 30] }));
        assert_eq!("10".parse::<TileIndices>(), Err(ParseTileIndicesError::TooShort));
    }

    #[test]
    fn test_deserialize_pff_header() {
        let src = r#"
        <PFFHEADER
            WIDTH="38843" HEIGHT="6700"
            NUMTILES="5541"
            NUMIMAGES="1"
            HEADERSIZE="15331"
            VERSION="106"
            ZA="0"
            TILESIZE="256"
        />"#;
        let props: PffHeader = serde_xml_rs::from_str(src).unwrap();
        assert_eq!(props.width, 38843);
        assert_eq!(props.height, 6700);
        assert_eq!(props.tile_size, 256);
        assert_eq!(props.num_tiles, 5541);
        assert_eq!(props.header_size, 15331);
        assert_eq!(props.version, 106);
        assert_eq!(props.index_bytes(), (16391, 60719));
    }

    #[test]
    fn test_deserialize_indices_reply() {
        let src = "Error=0&newSize=126&reply_data=1,  0  1  2";
        let reply: Reply<TileIndices> = serde_urlencoded::from_str(src).unwrap();
        assert_eq!(reply.reply_data.indices, vec![1, 2, 3]);
    }

    #[test]
    fn test_deserialize_pff_header_reply() {
        let src = r#"Error=0&newSize=126&reply_data=<PFFHEADER
            WIDTH="38843" HEIGHT="6700"
            NUMTILES="5541"
            NUMIMAGES="1"
            HEADERSIZE="15331"
            VERSION="106"
            ZA="0"
            TILESIZE="256"
        />"#;
        let reply: Reply<PffHeader> = serde_urlencoded::from_str(src).unwrap();
        assert_eq!(reply.reply_data.width, 38843);
    }
}