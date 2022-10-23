use std::num::ParseIntError;
use std::str::FromStr;

use serde::{Serialize, Deserialize, Deserializer};

use custom_error::custom_error;

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct Reply<T: FromStr> where <T as FromStr>::Err: ToString {
    #[serde(deserialize_with = "deserialize_from_string")]
    pub reply_data: T,
}

fn deserialize_from_string<'d, D: Deserializer<'d>, T: FromStr>(de: D) -> Result<T, D::Error>
    where <T as FromStr>::Err: ToString {
    let as_str = String::deserialize(de)?;
    FromStr::from_str(&as_str)
        .map_err(|e: <T as FromStr>::Err| serde::de::Error::custom(e.to_string()))
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone)]
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
    pub header_size: u64,
    #[serde(rename = "VERSION", default)]
    pub version: u32,
}


impl FromStr for PffHeader {
    type Err = serde_xml_rs::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_xml_rs::from_str(s)
    }
}

#[derive(Debug, Clone)]
pub struct HeaderInfo {
    pub base_url: String,
    pub file: String,
    pub header: PffHeader,
}

impl HeaderInfo {
    fn request_url(&self, params: ServletRequestParams) -> String {
        let params = params.with_file(&self.file);
        let params_str = serde_urlencoded::to_string(params).expect("parameters are encodable");
        format!("{}?{}", self.base_url, params_str)
    }

    pub fn tiles_index_url(&self) -> String {
        let header = &self.header;
        let begin = 0x424 + header.header_size;
        let end = begin + 8 * u64::from(header.num_tiles);
        self.request_url(ServletRequestParams {
            vers: header.version,
            head: header.header_size,
            begin,
            end,
            request_type: RequestType::TileIndices as u8,
        })
    }
}

#[derive(Debug)]
pub struct ImageInfo {
    pub header_info: HeaderInfo,
    pub tiles: TileIndices,
}

impl ImageInfo {
    pub fn tile_url(&self, tile_number: usize) -> String {
        let header = &self.header_info.header;
        let tiles = &self.tiles;
        let begin = if let Some(i) = tile_number.checked_sub(1) {
            tiles.indices[i]
        } else {
            0x424 + header.header_size + 8 * u64::from(header.num_tiles)
        };
        self.header_info.request_url(ServletRequestParams {
            vers: header.version,
            head: header.header_size,
            begin,
            end: tiles.indices[tile_number],
            request_type: RequestType::TileImage as u8,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct TileIndices {
    indices: Vec<u64>
}

custom_error! {#[derive(PartialEq, Eq)] pub ParseTileIndicesError
    TooShort = "Missing a part of tile indices string",
    BadNum{source: ParseIntError} = "Invalid tile index: {source}",
}

impl FromStr for TileIndices {
    type Err = ParseTileIndicesError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(',');
        let first: u64 = parts.next().ok_or(ParseTileIndicesError::TooShort)?.parse()?;
        let rest_str: &str = parts.next().ok_or(ParseTileIndicesError::TooShort)?;
        let other_nums = rest_str.split_ascii_whitespace();
        let indices = other_nums
            .map(|num_str| num_str.parse::<u64>().map(|x| first + x))
            .collect::<Result<Vec<u64>, _>>()?;
        Ok(TileIndices { indices })
    }
}

#[derive(PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum RequestType {
    TileImage = 0,
    Metadata = 1,
    TileIndices = 2,
}

#[derive(Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ServletRequestParams {
    //pff file version number
    vers: u32,
    //the size of the JFIF headers table
    head: u64,
    // a beginning offset in the pff file (in bytes)
    begin: u64,
    //end offset
    end: u64,
    // 0, 1 or 2, see RequestType
    pub request_type: u8,
}

impl ServletRequestParams {
    pub fn with_file(self, file: &str) -> FullServletRequestParams<'_> {
        FullServletRequestParams { file, params: self }
    }
}

#[derive(Deserialize, Serialize, PartialEq, Eq, Debug)]
pub struct FullServletRequestParams<'a> {
    file: &'a str,
    #[serde(flatten)]
    params: ServletRequestParams,
}

#[derive(Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InitialServletRequestParams {
    //the path to the pff file
    pub file: String,
    pub request_type: u8,
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
        let header: PffHeader = serde_xml_rs::from_str(src).unwrap();
        assert_eq!(header.width, 38843);
        assert_eq!(header.height, 6700);
        assert_eq!(header.tile_size, 256);
        assert_eq!(header.num_tiles, 5541);
        assert_eq!(header.header_size, 15331);
        assert_eq!(header.version, 106);
        let header_info = HeaderInfo { header, file: "x".into(), base_url: "http://x.com/".into() };
        assert_eq!(
            header_info.tiles_index_url(),
            "http://x.com/?file=x&vers=106&head=15331&begin=16391&end=60719&requestType=2"
        );
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