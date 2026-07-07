use std::borrow::{Borrow, Cow};
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Debug};
use std::str::FromStr;
use std::sync::Arc;

pub use crate::errors::DezoomerError;

pub use super::Vec2d;
use super::ZoomError;
use crate::dezoomer::PageContents::Success;

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
            PageContents::Error(e) => write!(f, "{e}"),
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
    pub fn with_contents(&self) -> Result<DezoomerInputWithContents<'_>, DezoomerError> {
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
pub type ZoomLevel = Box<dyn TileProvider + Send + Sync>;

/// A collection of multiple resolutions at which an image is available
pub type ZoomLevels = Vec<ZoomLevel>;

/// Represents a single zoomable image with multiple resolution levels
/// All the levels are already cheaply available synchronously.
pub trait ZoomableImageWithLevels: Send + Sync + std::fmt::Debug {
    /// Extract all available zoom levels for this image (consumes self)
    fn into_zoom_levels(self: Box<Self>) -> Result<ZoomLevels, DezoomerError>;

    /// Get a human-readable title for this image
    fn title(&self) -> Option<String>;
}

/// A URL that can be processed by dezoomers to create ZoomableImages
#[derive(Debug, Clone)]
pub struct ZoomableImageUrl {
    pub url: String,
    pub title: Option<String>,
}

/// Result type for dezoomer operations - a vector of ZoomableImages
pub type DezoomerResult = Vec<ZoomableImage>;

/// An image that can be asynchonously resolved to a ZoomableImageWithLevels
/// It already has the title, but no zoom levels available.
#[derive(Debug)]
pub enum ZoomableImage {
    /// Direct zoomable images (e.g., from IIIF manifests, krpano configs)
    Image(Box<dyn ZoomableImageWithLevels>),
    /// URLs that need further processing by other dezoomers
    ImageUrl(ZoomableImageUrl),
}

impl ZoomableImage {
    pub fn title(&self) -> Option<Cow<'_, str>> {
        match self {
            ZoomableImage::Image(image) => image.title().map(Cow::Owned),
            ZoomableImage::ImageUrl(url) => url.title.as_deref().map(Cow::Borrowed),
        }
    }

    pub async fn into_zoom_levels(
        self,
        http: &reqwest::Client,
    ) -> Result<ZoomLevels, DezoomerError> {
        match self {
            ZoomableImage::Image(image) => image.into_zoom_levels(),
            ZoomableImage::ImageUrl(url) => {
                // Import at the top of the function rather than globally
                use crate::auto::{all_dezoomers, prioritize_dezoomers_for_url};
                use crate::network::fetch_uri;
                use log::debug;

                let ZoomableImageUrl { url, title } = url;

                debug!("Resolving ZoomableImageUrl: {}", url);

                // Try each dezoomer on this URL to find one that can process it
                // Prioritize dezoomers based on URL patterns for better performance
                let dezoomers = prioritize_dezoomers_for_url(&url, all_dezoomers(false));

                for mut dezoomer in dezoomers {
                    debug!("Trying dezoomer '{}' on URL: {}", dezoomer.name(), url);

                    // Use the dezoomer's zoom_levels method to try to extract levels
                    let mut input = DezoomerInput {
                        uri: url.clone(),
                        contents: PageContents::Unknown,
                    };

                    // Handle the NeedsData loop
                    loop {
                        match dezoomer.zoom_levels(&input) {
                            Ok(levels) => {
                                debug!(
                                    "Dezoomer '{}' successfully extracted {} zoom levels",
                                    dezoomer.name(),
                                    levels.len()
                                );
                                return Ok(zoom_levels_with_title(levels, title));
                            }
                            Err(DezoomerError::NeedsData { uri: needed_uri }) => {
                                debug!(
                                    "Dezoomer '{}' needs data from: {}",
                                    dezoomer.name(),
                                    needed_uri
                                );
                                let contents = fetch_uri(&needed_uri, http).await.into();
                                input.uri = needed_uri;
                                input.contents = contents;
                            }
                            Err(e) => {
                                debug!("Dezoomer '{}' failed: {}", dezoomer.name(), e);
                                break; // Try next dezoomer
                            }
                        }
                    }
                }

                Err(DezoomerError::WrongDezoomer {
                    name: "No dezoomer could process this URL",
                })
            }
        }
    }
}

#[derive(Debug)]
struct TitledZoomLevel {
    inner: ZoomLevel,
    title: Arc<str>,
}

impl TileProvider for TitledZoomLevel {
    fn next_tiles(&mut self, previous: Option<TileFetchResult>) -> Vec<TileReference> {
        self.inner.next_tiles(previous)
    }

    fn post_process_fn(&self) -> PostProcessFn {
        self.inner.post_process_fn()
    }

    fn fmt_name(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_level_name(
            f,
            format_args!("{}", self.title),
            self.inner.size_hint(),
            self.inner.tile_count_hint(),
        )
    }

    fn title(&self) -> Option<String> {
        Some(self.title.to_string())
    }

    fn size_hint(&self) -> Option<Vec2d> {
        self.inner.size_hint()
    }

    fn tile_count_hint(&self) -> Option<u32> {
        self.inner.tile_count_hint()
    }

    fn tile_size_hint(&self) -> Option<Vec2d> {
        self.inner.tile_size_hint()
    }

    fn http_headers(&self) -> HashMap<String, String> {
        self.inner.http_headers()
    }
}

fn zoom_levels_with_title(levels: ZoomLevels, title: Option<String>) -> ZoomLevels {
    let Some(title) = title.filter(|title| !title.trim().is_empty()) else {
        return levels;
    };

    let title: Arc<str> = Arc::from(title);

    levels
        .into_iter()
        .map(|inner| {
            if inner.title().is_some_and(|title| !title.trim().is_empty()) {
                inner
            } else {
                Box::new(TitledZoomLevel {
                    inner,
                    title: title.clone(),
                }) as ZoomLevel
            }
        })
        .collect()
}

#[derive(Debug)]
pub struct SimpleZoomableImage {
    zoom_levels: ZoomLevels,
    title: Option<String>,
}

impl SimpleZoomableImage {
    pub fn new(zoom_levels: ZoomLevels, title: Option<String>) -> Self {
        SimpleZoomableImage { zoom_levels, title }
    }
}

impl ZoomableImageWithLevels for SimpleZoomableImage {
    fn into_zoom_levels(self: Box<Self>) -> Result<ZoomLevels, DezoomerError> {
        Ok(self.zoom_levels)
    }

    fn title(&self) -> Option<String> {
        self.title.clone()
    }
}

pub trait IntoZoomLevels {
    fn into_zoom_levels(self) -> ZoomLevels;
}

impl<I, Z> IntoZoomLevels for I
where
    I: Iterator<Item = Z>,
    Z: TileProvider + Send + Sync + 'static,
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

    /// Extract images or image URLs from the input data
    fn dezoomer_result(&mut self, data: &DezoomerInput) -> Result<DezoomerResult, DezoomerError> {
        let levels = self.zoom_levels(data)?;
        let image = SimpleZoomableImage::new(levels, None);
        Ok(dezoomer_result_from_single_image(image))
    }

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
        TileProviderName(self).to_string()
    }

    /// Format this provider for zoom-level pickers.
    fn fmt_name(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }

    /// The title of the image
    fn title(&self) -> Option<String> {
        None
    }

    /// The width and height of the image. Can be unknown when dezooming starts
    fn size_hint(&self) -> Option<Vec2d> {
        None
    }

    /// The number of tiles in the image. Can be unknown when dezooming starts
    fn tile_count_hint(&self) -> Option<u32> {
        None
    }

    fn tile_size_hint(&self) -> Option<Vec2d> {
        None
    }

    /// A collection of http headers to use when requesting the tiles
    fn http_headers(&self) -> HashMap<String, String> {
        HashMap::new()
    }
}

struct TileProviderName<'a, T: TileProvider + ?Sized>(&'a T);

impl<T: TileProvider + ?Sized> fmt::Display for TileProviderName<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt_name(f)
    }
}

impl fmt::Display for dyn TileProvider + Send + Sync + '_ {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_name(f)
    }
}

fn fmt_level_name(
    f: &mut fmt::Formatter<'_>,
    label: fmt::Arguments<'_>,
    size: Option<Vec2d>,
    tile_count: Option<u32>,
) -> fmt::Result {
    f.write_fmt(label)?;
    match (size, tile_count) {
        (Some(Vec2d { x, y }), Some(tile_count)) => {
            write!(f, " ({x:>5} x {y:>5} pixels, {tile_count:>5} tiles)")
        }
        (Some(Vec2d { x, y }), None) => write!(f, " ({x:>5} x {y:>5} pixels)"),
        (None, Some(tile_count)) => write!(f, " ({tile_count:>5} tiles)"),
        (None, None) => Ok(()),
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
        if tiles.is_empty() { None } else { Some(tiles) }
    }
    pub fn set_fetch_result(&mut self, result: TileFetchResult) {
        assert!(self.waiting_results);
        self.waiting_results = false;
        self.previous = Some(result)
    }
    pub fn size_hint(&self) -> Option<Vec2d> {
        self.zoom_level.size_hint()
    }
    pub fn tile_size_hint(&self) -> Option<Vec2d> {
        self.zoom_level.tile_size_hint()
    }
}

/// Shortcut to return a single zoom level from a dezoomer
pub fn single_level<T: TileProvider + Send + Sync + 'static>(
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

    fn fmt_name(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_level_name(
            f,
            format_args!("{self:?}"),
            Some(self.size()),
            Some(self.tile_count()),
        )
    }

    fn title(&self) -> Option<String> {
        TilesRect::title(self)
    }

    fn size_hint(&self) -> Option<Vec2d> {
        Some(self.size())
    }

    fn tile_count_hint(&self) -> Option<u32> {
        Some(self.tile_count())
    }

    fn tile_size_hint(&self) -> Option<Vec2d> {
        Some(self.tile_size())
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

/// Helper functions for creating DezoomerResult from common types
///
/// Convert a vector of ZoomableImageUrl to DezoomerResult
pub fn dezoomer_result_from_urls(urls: Vec<ZoomableImageUrl>) -> DezoomerResult {
    urls.into_iter().map(ZoomableImage::ImageUrl).collect()
}

/// Convert a vector of ZoomableImageWithLevels to DezoomerResult
pub fn dezoomer_result_from_images(
    images: Vec<Box<dyn ZoomableImageWithLevels>>,
) -> DezoomerResult {
    images.into_iter().map(ZoomableImage::Image).collect()
}

/// Convert a single ZoomableImageWithLevels to DezoomerResult
pub fn dezoomer_result_from_single_image<T: ZoomableImageWithLevels + 'static>(
    image: T,
) -> DezoomerResult {
    vec![ZoomableImage::Image(Box::new(image))]
}

/// Convert a single ZoomableImageUrl to DezoomerResult
pub fn dezoomer_result_from_single_url(url: ZoomableImageUrl) -> DezoomerResult {
    vec![ZoomableImage::ImageUrl(url)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeLvl {
        title: Option<&'static str>,
    }

    impl std::fmt::Debug for FakeLvl {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("FakeLvl")
        }
    }

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

        fn title(&self) -> Option<String> {
            self.title.map(str::to_string)
        }
    }

    #[test]
    fn assert_tiles() {
        let mut lvl: ZoomLevel = Box::<FakeLvl>::default();
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

    #[test]
    fn test_simple_zoomable_image() {
        let zoom_levels: ZoomLevels = vec![Box::<FakeLvl>::default()];
        let title = Some("Test Image".to_string());

        let image = SimpleZoomableImage::new(zoom_levels, title.clone());

        // Test title retrieval
        assert_eq!(image.title(), title);

        // Test that the image can be used as a ZoomableImage trait object
        let boxed_image: Box<dyn ZoomableImageWithLevels> = Box::new(image);
        assert_eq!(boxed_image.title(), title);

        // Test that into_zoom_levels works correctly
        let extracted_levels = boxed_image.into_zoom_levels().unwrap();
        assert_eq!(extracted_levels.len(), 1);
    }

    #[test]
    fn test_zoom_levels_with_title_preserves_level_details() {
        let levels =
            zoom_levels_with_title(vec![Box::<FakeLvl>::default()], Some("Readable".into()));

        let display_name = format!("{}", &*levels[0]);
        assert_eq!(levels[0].title(), Some("Readable".to_string()));
        assert_eq!(levels[0].name(), display_name);
        assert!(display_name.starts_with("Readable ("));
        assert!(display_name.contains("pixels"));
        assert!(display_name.contains("tiles"));
    }

    #[test]
    fn test_zoom_levels_with_title_preserves_inner_title() {
        let levels = zoom_levels_with_title(
            vec![Box::new(FakeLvl {
                title: Some("Inner Title"),
            })],
            Some("Outer Title".into()),
        );

        assert_eq!(levels[0].title(), Some("Inner Title".to_string()));
        assert!(format!("{}", &*levels[0]).starts_with("FakeLvl ("));
    }
}
