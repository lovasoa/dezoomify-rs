//! Types for discovering logical images and their tiled resolution levels.
//!
//! A dezoomer returns [`Images`] rather than a flat level list. Most formats
//! contain one image and can finish their implementation with
//! `Ok(levels.into())`. Container formats such as krpano and IIIF manifests
//! return one [`ZoomableImage`] per scene or referenced image.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Debug};
use std::str::FromStr;

pub use crate::errors::DezoomerError;

pub use super::Vec2d;
use super::ZoomError;
use crate::dezoomer::PageContents::Success;

#[cfg(test)]
pub(crate) mod test_utils;

pub enum PageContents {
    Unknown,
    Success(Vec<u8>),
    Error(ZoomError),
}

impl From<Result<Vec<u8>, ZoomError>> for PageContents {
    fn from(res: Result<Vec<u8>, ZoomError>) -> Self {
        res.map_or_else(Self::Error, Self::Success)
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
    /// Returns the downloaded contents or an error describing why they are unavailable.
    ///
    /// # Errors
    ///
    /// Returns [`DezoomerError::NeedsData`] when the resource has not been fetched, or
    /// [`DezoomerError::DownloadError`] when fetching failed.
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

/// A single logical image whose zoom levels are already available.
#[derive(Debug)]
pub struct ResolvedImage {
    zoom_levels: ZoomLevels,
    title: Option<String>,
}

impl ResolvedImage {
    #[must_use]
    pub fn new(zoom_levels: ZoomLevels, title: Option<String>) -> Self {
        Self { zoom_levels, title }
    }

    #[must_use]
    pub fn into_zoom_levels(self) -> ZoomLevels {
        self.zoom_levels
    }

    #[must_use]
    pub fn levels(&self) -> &[ZoomLevel] {
        &self.zoom_levels
    }

    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    fn with_fallback_title(mut self, title: Option<String>) -> Self {
        if self.title.as_deref().is_none_or(str::is_empty) {
            self.title = title;
        }
        self
    }
}

/// A deferred logical image that must be processed by a dezoomer.
#[derive(Debug, Clone)]
pub struct ImageUrl {
    pub url: String,
    pub title: Option<String>,
}

/// Logical images discovered by a dezoomer.
///
/// [`ZoomLevels`] converts into a collection containing one resolved image.
/// Vectors of [`ResolvedImage`] or [`ImageUrl`] preserve every logical image.
#[derive(Debug, Default)]
pub struct Images(Vec<ZoomableImage>);

impl Images {
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, ZoomableImage> {
        self.0.iter()
    }

    fn with_fallback_title(self, title: Option<String>) -> Self {
        let Some(title) = title.filter(|title| !title.trim().is_empty()) else {
            return self;
        };

        Self(
            self.0
                .into_iter()
                .map(|image| match image {
                    ZoomableImage::Resolved(image) => {
                        ZoomableImage::Resolved(image.with_fallback_title(Some(title.clone())))
                    }
                    ZoomableImage::Url(mut image_url) => {
                        if image_url.title.as_deref().is_none_or(str::is_empty) {
                            image_url.title = Some(title.clone());
                        }
                        ZoomableImage::Url(image_url)
                    }
                })
                .collect(),
        )
    }
}

impl IntoIterator for Images {
    type Item = ZoomableImage;
    type IntoIter = std::vec::IntoIter<ZoomableImage>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Images {
    type Item = &'a ZoomableImage;
    type IntoIter = std::slice::Iter<'a, ZoomableImage>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl std::ops::Index<usize> for Images {
    type Output = ZoomableImage;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl From<ZoomLevels> for Images {
    fn from(levels: ZoomLevels) -> Self {
        ResolvedImage::new(levels, None).into()
    }
}

impl From<ResolvedImage> for Images {
    fn from(image: ResolvedImage) -> Self {
        Self(vec![ZoomableImage::Resolved(image)])
    }
}

impl From<Vec<ResolvedImage>> for Images {
    fn from(images: Vec<ResolvedImage>) -> Self {
        Self(images.into_iter().map(ZoomableImage::Resolved).collect())
    }
}

impl From<Vec<ImageUrl>> for Images {
    fn from(urls: Vec<ImageUrl>) -> Self {
        Self(urls.into_iter().map(ZoomableImage::Url).collect())
    }
}

impl From<Vec<ZoomableImage>> for Images {
    fn from(images: Vec<ZoomableImage>) -> Self {
        Self(images)
    }
}

impl FromIterator<ZoomableImage> for Images {
    fn from_iter<T: IntoIterator<Item = ZoomableImage>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// A logical image, either resolved or represented by a URL to resolve.
#[derive(Debug)]
pub enum ZoomableImage {
    /// An image whose levels are ready to use.
    Resolved(ResolvedImage),
    /// A URL that needs further processing.
    Url(ImageUrl),
}

impl ZoomableImage {
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        match self {
            ZoomableImage::Resolved(image) => image.title(),
            ZoomableImage::Url(url) => url.title.as_deref(),
        }
    }

    /// Resolves a deferred image URL, if necessary.
    ///
    /// # Errors
    ///
    /// Returns an error if metadata cannot be downloaded or interpreted by any dezoomer.
    pub async fn resolve(self, http: &reqwest::Client) -> Result<Images, DezoomerError> {
        let mut resolver = crate::auto::MetadataResolver::new(http);
        self.resolve_with(&mut resolver).await
    }

    pub(crate) async fn resolve_with(
        self,
        resolver: &mut crate::auto::MetadataResolver<'_>,
    ) -> Result<Images, DezoomerError> {
        match self {
            ZoomableImage::Resolved(image) => Ok(image.into()),
            ZoomableImage::Url(url) => {
                use crate::auto::AutoDezoomer;
                use log::debug;

                let ImageUrl { url, title } = url;

                debug!("Resolving image URL: {url}");
                let mut dezoomer = AutoDezoomer::default();
                let images = resolver.resolve(&mut dezoomer, &url).await?;
                debug!("Successfully extracted {} images", images.len());
                Ok(images.with_fallback_title(title))
            }
        }
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

/// Discovers logical zoomable images from downloaded metadata.
pub trait Dezoomer {
    /// The name of the image format. Used for dezoomer selection
    fn name(&self) -> &'static str;

    /// Discover logical images without flattening their zoom levels.
    ///
    /// Return [`DezoomerError::NeedsData`] when another resource must be
    /// downloaded, preserving any parser state needed for the next call.
    ///
    /// # Errors
    ///
    /// Returns an error when the input is unavailable, belongs to another format,
    /// requires another resource, or contains invalid metadata.
    fn images(&mut self, data: &DezoomerInput) -> Result<Images, DezoomerError>;

    /// Verifies a format-specific condition.
    ///
    /// # Errors
    ///
    /// Returns [`DezoomerError::WrongDezoomer`] when `c` is false.
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
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.tile_size
            .as_ref()
            .is_some_and(|&Vec2d { x, y }| x > 0 && y > 0)
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
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the formatter fails.
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

    fn scale_factor_hint(&self) -> Option<u32> {
        None
    }

    fn has_overlapping_tiles(&self) -> bool {
        false
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
    /// Returns the next batch of tile references.
    ///
    /// # Panics
    ///
    /// Panics if the previous batch has not been followed by [`Self::set_fetch_result`].
    pub fn next_tile_references(&mut self) -> Option<Vec<TileReference>> {
        assert!(!self.waiting_results);
        self.waiting_results = true;
        let tiles = self.zoom_level.next_tiles(self.previous);
        if tiles.is_empty() { None } else { Some(tiles) }
    }
    /// Records the result of fetching the previous batch.
    ///
    /// # Panics
    ///
    /// Panics if [`Self::next_tile_references`] has not produced a pending batch.
    pub fn set_fetch_result(&mut self, result: TileFetchResult) {
        assert!(self.waiting_results);
        self.waiting_results = false;
        self.previous = Some(result);
    }
    #[must_use]
    pub fn size_hint(&self) -> Option<Vec2d> {
        self.zoom_level.size_hint()
    }
    #[must_use]
    pub fn tile_size_hint(&self) -> Option<Vec2d> {
        self.zoom_level.tile_size_hint()
    }
    #[must_use]
    pub fn scale_factor_hint(&self) -> Option<u32> {
        self.zoom_level.scale_factor_hint()
    }
    #[must_use]
    pub fn has_overlapping_tiles(&self) -> bool {
        self.zoom_level.has_overlapping_tiles()
    }
}

/// Shortcut to return a single zoom level from a dezoomer
pub fn single_level<T: TileProvider + Send + Sync + 'static>(level: T) -> ZoomLevels {
    vec![Box::new(level)]
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

    fn has_overlapping_tiles(&self) -> bool {
        false
    }

    fn scale_factor_hint(&self) -> Option<u32> {
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

    fn scale_factor_hint(&self) -> Option<u32> {
        TilesRect::scale_factor_hint(self)
    }

    fn has_overlapping_tiles(&self) -> bool {
        TilesRect::has_overlapping_tiles(self)
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
    fn test_resolved_image() {
        let zoom_levels: ZoomLevels = vec![Box::<FakeLvl>::default()];
        let title = Some("Test Image".to_string());

        let image = ResolvedImage::new(zoom_levels, title.clone());

        assert_eq!(image.title(), title.as_deref());
        let extracted_levels = image.into_zoom_levels();
        assert_eq!(extracted_levels.len(), 1);
    }

    #[test]
    fn zoom_levels_convert_to_one_resolved_image() {
        let images: Images = vec![Box::<FakeLvl>::default() as ZoomLevel].into();

        let image = test_utils::expect_single_resolved(images);
        assert_eq!(image.into_zoom_levels().len(), 1);
    }

    #[test]
    fn fallback_title_does_not_replace_image_title() {
        let images = Images::from(vec![
            ResolvedImage::new(vec![], None),
            ResolvedImage::new(vec![], Some("Child".into())),
        ])
        .with_fallback_title(Some("Parent".into()));
        let titles = images
            .iter()
            .map(|image| image.title().map(str::to_string))
            .collect::<Vec<_>>();

        assert_eq!(titles, vec![Some("Parent".into()), Some("Child".into())]);
    }
}
