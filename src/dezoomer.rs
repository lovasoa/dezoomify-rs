use std::borrow::Borrow;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::Debug;
use std::str::FromStr;

pub use crate::errors::DezoomerError;

pub use super::Vec2d;
use super::ZoomError;
use crate::dezoomer::PageContents::Success;
use std::fmt;

pub enum PageContents {
    Unknown,
    Success(Vec<u8>),
    Error(ZoomError),
}

impl From<Result<Vec<u8>, ZoomError>> for PageContents {
    fn from(res: Result<Vec<u8>, ZoomError>) -> Self {
        res.map(Self::Success).unwrap_or_else(Self::Error)
    }
}

impl std::fmt::Debug for PageContents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => f.write_str("<not yet available>"),
            Success(contents) => f.write_str(&String::from_utf8_lossy(contents)),
            PageContents::Error(e) => write!(f, "{}", e),
        }
    }
}

pub struct DezoomerInput {
    pub uri: String,
    pub contents: PageContents,
}

pub struct DezoomerInputWithContents<'a> {
    pub uri: &'a str,
    pub contents: &'a [u8],
}

impl DezoomerInput {
    pub fn with_contents(&self) -> Result<DezoomerInputWithContents, DezoomerError> {
        match &self.contents {
            PageContents::Unknown => Err(DezoomerError::NeedsData {
                uri: self.uri.clone(),
            }),
            Success(contents) => Ok(DezoomerInputWithContents {
                uri: &self.uri,
                contents,
            }),
            PageContents::Error(e) => Err(DezoomerError::DownloadError { msg: e.to_string() }),
        }
    }
}

/// A single image with a given width and height
pub type ZoomLevel = Box<dyn TileProvider + Sync>;

/// A collection of multiple resolutions at which an image is available
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

/// A trait that should be implemented by every zoomable image dezoomer
pub trait Dezoomer {
    /// The name of the image format. Used for dezoomer selection
    fn name(&self) -> &'static str;

    /// List of the various sizes at which an image is available
    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError>;
    fn assert(&self, c: bool) -> Result<(), DezoomerError> {
        if c {
            Ok(())
        } else {
            Err(self.wrong_dezoomer())
        }
    }
    fn wrong_dezoomer(&self) -> DezoomerError {
        DezoomerError::WrongDezoomer { name: self.name() }
    }
}

#[derive(Clone, Copy)]
pub struct TileFetchResult {
    pub count: u64,
    pub successes: u64,
    pub tile_size: Option<Vec2d>,
}

impl TileFetchResult {
    pub fn is_success(&self) -> bool {
        self.tile_size
            .filter(|&Vec2d { x, y }| x > 0 && y > 0)
            .is_some()
            && self.successes > 0
    }
}

type PostProcessResult = Result<Vec<u8>, Box<dyn Error + Send>>;
// TODO : fix
// see: https://github.com/rust-lang/rust/issues/63033
#[derive(Clone, Copy)]
pub enum PostProcessFn {
    Fn(fn(&TileReference, Vec<u8>) -> PostProcessResult),
    None,
}

/// A single tiled image
pub trait TileProvider: Debug {
    /// Provide a list of image tiles. Should be called repetitively until it returns
    /// an empty list. Each new call takes the results of the previous tile fetch as a parameter.
    fn next_tiles(&mut self, previous: Option<TileFetchResult>) -> Vec<TileReference>;

    /// A function that takes the downloaded tile bytes and decodes them
    fn post_process_fn(&self) -> PostProcessFn {
        PostProcessFn::None
    }

    /// The name of the format
    fn name(&self) -> String {
        format!("{:?}", self)
    }

    /// The title of the image
    fn title(&self) -> Option<String> {
        None
    }

    /// The width and height of the image. Can be unknown when dezooming starts
    fn size_hint(&self) -> Option<Vec2d> {
        None
    }

    /// A collection of http headers to use when requesting the tiles
    fn http_headers(&self) -> HashMap<String, String> {
        HashMap::new()
    }
}

/// Used to iterate over all the batches of tiles in a zoom level
pub struct ZoomLevelIter<'a> {
    zoom_level: &'a mut ZoomLevel,
    previous: Option<TileFetchResult>,
    waiting_results: bool,
}

impl<'a> ZoomLevelIter<'a> {
    pub fn new(zoom_level: &'a mut ZoomLevel) -> Self {
        ZoomLevelIter {
            zoom_level,
            previous: None,
            waiting_results: false,
        }
    }
    pub fn next_tile_references(&mut self) -> Option<Vec<TileReference>> {
        assert!(!self.waiting_results);
        self.waiting_results = true;
        let tiles = self.zoom_level.next_tiles(self.previous);
        if tiles.is_empty() {
            None
        } else {
            Some(tiles)
        }
    }
    pub fn set_fetch_result(&mut self, result: TileFetchResult) {
        assert!(self.waiting_results);
        self.waiting_results = false;
        self.previous = Some(result)
    }
    pub fn size_hint(&self) -> Option<Vec2d> {
        self.zoom_level.size_hint()
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
    fn title(&self) -> Option<String> {
        None
    }
    fn tile_ref(&self, pos: Vec2d) -> TileReference {
        TileReference {
            url: self.tile_url(pos),
            position: self.tile_size() * pos,
        }
    }
    fn post_process_fn(&self) -> PostProcessFn {
        PostProcessFn::None
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
        let this: &T = self.borrow(); // Immutable borrow
        (0..h)
            .flat_map(move |y| (0..w).map(move |x| this.tile_ref(Vec2d { x, y })))
            .collect()
    }

    fn post_process_fn(&self) -> PostProcessFn {
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

    fn title(&self) -> Option<String> {
        TilesRect::title(self)
    }

    fn size_hint(&self) -> Option<Vec2d> {
        Some(self.size())
    }

    fn http_headers(&self) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        // By default, use the first tile as the referer, so that it is on the same domain
        headers.insert("Referer".into(), self.tile_url(Vec2d::default()));
        headers
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
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

impl fmt::Display for TileReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.url)
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
        let mut zoom_level_iter = ZoomLevelIter::new(&mut lvl);
        while let Some(tiles) = zoom_level_iter.next_tile_references() {
            all_tiles.extend(tiles);
            zoom_level_iter.set_fetch_result(TileFetchResult {
                count: 0,
                successes: 0,
                tile_size: None,
            });
        }
        assert_eq!(
            all_tiles,
            vec![
                TileReference {
                    url: "0,0".into(),
                    position: Vec2d { x: 0, y: 0 },
                },
                TileReference {
                    url: "1,0".into(),
                    position: Vec2d { x: 60, y: 0 },
                },
                TileReference {
                    url: "0,1".into(),
                    position: Vec2d { x: 0, y: 60 },
                },
                TileReference {
                    url: "1,1".into(),
                    position: Vec2d { x: 60, y: 60 },
                }
            ]
        );
    }
}
