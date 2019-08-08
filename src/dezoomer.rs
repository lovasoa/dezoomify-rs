use std::collections::HashMap;
use std::error::Error;
use std::fmt::Debug;
use std::ops::{Add, Div, Mul};
use std::str::FromStr;

use custom_error::custom_error;

use super::ZoomError;

pub struct DezoomerInput {
    pub uri: String,
    pub contents: Option<Vec<u8>>,
}

pub struct DezoomerInputWithContents<'a> {
    pub uri: &'a str,
    pub contents: &'a [u8],
}

impl DezoomerInput {
    pub fn with_contents(&self) -> Result<DezoomerInputWithContents, DezoomerError> {
        if let Some(contents) = &self.contents {
            Ok(DezoomerInputWithContents { uri: &self.uri, contents })
        } else {
            Err(DezoomerError::NeedsData { uri: self.uri.clone() })
        }
    }
}

custom_error! {pub DezoomerError
    NeedsData{uri: String}           = "Need to download data from {uri}",
    WrongDezoomer{name:&'static str} = "The '{name}' dezoomer cannot handle this URI",
    Other{source: Box<dyn Error>}    = "Unable to create the dezoomer: {source}"
}

impl DezoomerError {
    pub fn wrap<E: Error + 'static>(err: E) -> DezoomerError {
        DezoomerError::Other { source: err.into() }
    }
}

pub type ZoomLevel = Box<dyn TileProvider + Sync>;
pub type ZoomLevels = Vec<ZoomLevel>;

pub trait Dezoomer {
    fn name(&self) -> &'static str;
    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError>;
    fn assert(&self, c: bool) -> Result<(), DezoomerError> {
        if c { Ok(()) } else {
            Err(DezoomerError::WrongDezoomer { name: self.name() })
        }
    }
}

pub trait TileProvider: Debug {
    fn tiles(&self) -> Vec<Result<TileReference, Box<dyn Error>>>;
    fn post_process_tile(&self, _tile: &TileReference, data: Vec<u8>) -> Result<Vec<u8>, Box<dyn Error>> {
        Ok(data)
    }
    fn name(&self) -> String {
        format!("{:?}", self)
    }
    fn http_headers(&self) -> HashMap<String, String> {
        HashMap::new()
    }
}

/// Shortcut to return a single zoom level from a dezoomer
pub fn single_level<T: TileProvider + Sync + 'static>(level: T)
                                                      -> Result<ZoomLevels, DezoomerError> {
    Ok(vec![Box::new(level)])
}

pub trait TilesRect: Debug {
    fn size(&self) -> Vec2d;
    fn tile_size(&self) -> Vec2d;
    fn tile_url(&self, pos: Vec2d) -> String;
    fn post_process_tile(&self, _tile: &TileReference, data: Vec<u8>) -> Result<Vec<u8>, Box<dyn Error>> {
        Ok(data)
    }
}

impl<T: TilesRect> TileProvider for T {
    fn tiles(&self) -> Vec<Result<TileReference, Box<dyn Error>>> {
        let tile_size = self.tile_size();
        let Vec2d { x: w, y: h } = self.size() / tile_size;

        (0..w).flat_map(move |x| {
            (0..h).map(move |y| {
                let position = Vec2d { x, y };
                let url = self.tile_url(position);
                Ok(TileReference { url, position: position * tile_size })
            })
        }).collect()
    }
    fn post_process_tile(&self, tile: &TileReference, data: Vec<u8>) -> Result<Vec<u8>, Box<dyn Error>> {
        TilesRect::post_process_tile(self, tile, data)
    }
    fn name(&self) -> String {
        let Vec2d { x, y } = self.size();
        format!("{:?} ({}x{})", self, x, y)
    }
}

#[derive(Debug, PartialEq, Default, Clone, Copy)]
pub struct Vec2d {
    pub x: u32,
    pub y: u32,
}

impl Vec2d {
    pub fn max(self, other: Vec2d) -> Vec2d {
        Vec2d {
            x: self.x.max(other.x),
            y: self.y.max(other.y),
        }
    }
}

impl std::fmt::Display for Vec2d {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "x={} y={}", self.x, self.y)
    }
}

impl Add<Vec2d> for Vec2d {
    type Output = Vec2d;

    fn add(self, rhs: Vec2d) -> Self::Output {
        Vec2d { x: self.x + rhs.x, y: self.y + rhs.y }
    }
}

impl Mul<Vec2d> for Vec2d {
    type Output = Vec2d;

    fn mul(self, rhs: Vec2d) -> Self::Output {
        Vec2d { x: self.x * rhs.x, y: self.y * rhs.y }
    }
}

impl Div<Vec2d> for Vec2d {
    type Output = Vec2d;

    fn div(self, rhs: Vec2d) -> Self::Output {
        Vec2d { x: self.x / rhs.x, y: self.y / rhs.y }
    }
}


#[derive(Debug, PartialEq, Clone)]
pub struct TileReference {
    pub url: String,
    pub position: Vec2d,
}

impl FromStr for TileReference {
    type Err = ZoomError;

    fn from_str(tile_str: &str) -> Result<Self, Self::Err> {
        let mut parts = tile_str.split(' ');
        let make_error = || ZoomError::MalformedTileStr { tile_str: String::from(tile_str) };

        if let (Some(x), Some(y), Some(url)) = (parts.next(), parts.next(), parts.next()) {
            let x: u32 = x.parse().map_err(|_| make_error())?;
            let y: u32 = y.parse().map_err(|_| make_error())?;
            Ok(TileReference {
                url: String::from(url),
                position: Vec2d { x, y },
            })
        } else {
            Err(make_error())
        }
    }
}