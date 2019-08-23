use std::borrow::Borrow;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::Debug;
use std::str::FromStr;

use custom_error::custom_error;

pub use super::Vec2d;
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
            Ok(DezoomerInputWithContents {
                uri: &self.uri,
                contents,
            })
        } else {
            Err(DezoomerError::NeedsData {
                uri: self.uri.clone(),
            })
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

pub trait IntoZoomLevels {
    fn into_zoom_levels(self) -> ZoomLevels;
}

impl<I, Z> IntoZoomLevels for I
where
    I: Iterator<Item = Z>,
    Z: TileProvider + Sync + 'static,
{
    fn into_zoom_levels(self) -> ZoomLevels {
        self.map(|x| Box::new(x) as ZoomLevel).collect()
    }
}

pub trait Dezoomer {
    fn name(&self) -> &'static str;
    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError>;
    fn assert(&self, c: bool) -> Result<(), DezoomerError> {
        if c {
            Ok(())
        } else {
            Err(DezoomerError::WrongDezoomer { name: self.name() })
        }
    }
}

pub struct TileFetchResult {
    pub count: u64,
    pub successes: u64,
    pub tile_size: Option<Vec2d>,
}

pub type PostProcessFn = fn(tile: &TileReference, data: Vec<u8>) -> Result<Vec<u8>, Box<dyn Error>>;

pub trait TileProvider: Debug {
    fn next_tiles(&mut self, previous: Option<TileFetchResult>) -> Vec<TileReference>;
    fn post_process_fn(&self) -> Option<PostProcessFn> {
        None
    }

    fn name(&self) -> String {
        format!("{:?}", self)
    }
    fn size_hint(&self) -> Option<Vec2d> {
        None
    }
    fn http_headers(&self) -> HashMap<String, String> {
        HashMap::new()
    }
}

/// Takes a zoom level and a function, and applies the function to all the batches of tiles
/// in the level
pub fn apply_to_tiles<F>(lvl: &mut ZoomLevel, mut downloader: F)
    where
        F: FnMut(Vec<TileReference>) -> TileFetchResult,
{
    let mut previous = None;
    while let Some(tiles) = Some(lvl.next_tiles(previous)).filter(|v| !v.is_empty()) {
        previous = Some(downloader(tiles))
    }
}

/// Shortcut to return a single zoom level from a dezoomer
pub fn single_level<T: TileProvider + Sync + 'static>(
    level: T,
) -> Result<ZoomLevels, DezoomerError> {
    Ok(vec![Box::new(level)])
}

pub trait TilesRect: Debug {
    fn size(&self) -> Vec2d;
    fn tile_size(&self) -> Vec2d;
    fn tile_url(&self, pos: Vec2d) -> String;
    fn post_process_fn(&self) -> Option<PostProcessFn> {
        None
    }
    fn tile_count(&self) -> u32 {
        let Vec2d { x, y } = self.size().ceil_div(self.tile_size());
        x * y
    }
}

impl<T: TilesRect> TileProvider for T {
    fn next_tiles(&mut self, previous: Option<TileFetchResult>) -> Vec<TileReference> {
        // When the dimensions are known in advance, we can always generate
        // a single batch of tile references. So any subsequent call returns an empty vector.
        if previous.is_some() {
            return vec![];
        }

        let tile_size = self.tile_size();
        let Vec2d { x: w, y: h } = self.size().ceil_div(tile_size);
        let this: &T = self.borrow();
        (0..h).flat_map(move |y| {
            (0..w).map(move |x| {
                    let position = Vec2d { x, y };
                let url = this.tile_url(position);
                TileReference {
                        url,
                        position: position * tile_size,
                }
                })
            })
            .collect()
    }

    fn post_process_fn(&self) -> Option<PostProcessFn> {
        TilesRect::post_process_fn(self)
    }

    fn name(&self) -> String {
        let Vec2d { x, y } = self.size();
        format!(
            "{:?} ({:>5} x {:>5} pixels, {:>5} tiles)",
            self,
            x,
            y,
            self.tile_count()
        )
    }
    fn size_hint(&self) -> Option<Vec2d> {
        Some(self.size())
    }
}

pub fn max_size_in_rect(position: Vec2d, tile_size: Vec2d, canvas_size: Vec2d) -> Vec2d {
    (position + tile_size).min(canvas_size) - position
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
        let make_error = || ZoomError::MalformedTileStr {
            tile_str: String::from(tile_str),
        };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FakeLvl;

    impl TilesRect for FakeLvl {
        fn size(&self) -> Vec2d {
            Vec2d { x: 100, y: 100 }
        }

        fn tile_size(&self) -> Vec2d {
            Vec2d { x: 60, y: 60 }
        }

        fn tile_url(&self, pos: Vec2d) -> String {
            format!("{},{}", pos.x, pos.y)
        }
    }

    #[test]
    fn assert_tiles() {
        let mut lvl: ZoomLevel = Box::new(FakeLvl {});
        let mut all_tiles = vec![];
        apply_to_tiles(&mut lvl, |tiles| {
            all_tiles.extend(tiles);
            TileFetchResult { count: 0, successes: 0, tile_size: None }
        });
        assert_eq!(all_tiles, vec![
            TileReference { url: "0,0".into(), position: Vec2d { x: 0, y: 0 } },
            TileReference { url: "1,0".into(), position: Vec2d { x: 60, y: 0 } },
            TileReference { url: "0,1".into(), position: Vec2d { x: 0, y: 60 } },
            TileReference { url: "1,1".into(), position: Vec2d { x: 60, y: 60 } }
        ]);
    }
}