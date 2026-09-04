//! Closed tile-source descriptions used by every zoom level.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use crate::Vec2d;

use super::adaptive::{AdaptiveSource, DiscoverableGrid};
use super::model::{ProcessingRecipe, Request, StableId, TileId, TileRole, TileSpec};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TileSourceError {
    ZeroImageDimensions,
    ZeroTileDimensions,
    ArithmeticOverflow,
    InvalidTile(String),
    InvalidDimensions,
}

impl fmt::Display for TileSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroImageDimensions => f.write_str("image dimensions must be greater than zero"),
            Self::ZeroTileDimensions => f.write_str("tile dimensions must be greater than zero"),
            Self::ArithmeticOverflow => f.write_str("tile geometry overflowed u32"),
            Self::InvalidTile(message) => f.write_str(message),
            Self::InvalidDimensions => f.write_str("a tile has invalid dimensions"),
        }
    }
}

impl Error for TileSourceError {}

/// A meaningful coordinate in a grid's standardized row-major domain.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GridCoord {
    pub column: u32,
    pub row: u32,
}

impl From<GridCoord> for Vec2d {
    fn from(value: GridCoord) -> Self {
        Self {
            x: value.column,
            y: value.row,
        }
    }
}

/// Geometry and identity of one cell in a validated grid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridTile {
    pub coord: GridCoord,
    pub row_major_ordinal: u64,
    pub image_size: Vec2d,
    pub cell_size: Vec2d,
    pub cell_extent: Vec2d,
    pub destination: Vec2d,
    pub expected_size: Vec2d,
}

/// Format-specific request behavior for a geometrically known grid.
pub trait GridRequests: fmt::Debug + Send + Sync {
    fn request(&self, tile: GridTile) -> Request;

    fn use_first_tile_as_referer(&self) -> bool {
        true
    }

    fn processing(&self) -> ProcessingRecipe {
        ProcessingRecipe::None
    }
}

struct ClosureRequests<F> {
    request: F,
    processing: ProcessingRecipe,
}

impl<F> fmt::Debug for ClosureRequests<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("request closure")
    }
}

impl<F: Fn(GridTile) -> Request + Send + Sync> GridRequests for ClosureRequests<F> {
    fn request(&self, tile: GridTile) -> Request {
        (self.request)(tile)
    }

    fn processing(&self) -> ProcessingRecipe {
        self.processing.clone()
    }
}

#[derive(Clone)]
pub struct Grid {
    level: StableId,
    image_size: Vec2d,
    tile_size: Vec2d,
    overlap: Vec2d,
    shape: Vec2d,
    count: u64,
    requests: Arc<dyn GridRequests>,
}

impl fmt::Debug for Grid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Grid")
            .field("level", &self.level)
            .field("image_size", &self.image_size)
            .field("tile_size", &self.tile_size)
            .field("overlap", &self.overlap)
            .field("shape", &self.shape)
            .field("count", &self.count)
            .field("requests", &self.requests)
            .finish()
    }
}

impl Grid {
    pub fn new(
        level: StableId,
        image_size: Vec2d,
        tile_size: Vec2d,
        overlap: Vec2d,
        requests: impl GridRequests + 'static,
    ) -> Result<Self, TileSourceError> {
        if image_size.x == 0 || image_size.y == 0 {
            return Err(TileSourceError::ZeroImageDimensions);
        }
        if tile_size.x == 0 || tile_size.y == 0 {
            return Err(TileSourceError::ZeroTileDimensions);
        }
        let shape = image_size.ceil_div(tile_size);
        let count = u64::from(shape.x)
            .checked_mul(u64::from(shape.y))
            .ok_or(TileSourceError::ArithmeticOverflow)?;
        // Validate every multiplication used by iteration up front.
        shape
            .x
            .saturating_sub(1)
            .checked_mul(tile_size.x)
            .ok_or(TileSourceError::ArithmeticOverflow)?;
        shape
            .y
            .saturating_sub(1)
            .checked_mul(tile_size.y)
            .ok_or(TileSourceError::ArithmeticOverflow)?;
        Ok(Self {
            level,
            image_size,
            tile_size,
            overlap,
            shape,
            count,
            requests: Arc::new(requests),
        })
    }

    pub fn with_requests(
        level: StableId,
        image_size: Vec2d,
        tile_size: Vec2d,
        overlap: Vec2d,
        requests: impl Fn(GridTile) -> Request + Send + Sync + 'static,
    ) -> Result<Self, TileSourceError> {
        Self::with_processed_requests(
            level,
            image_size,
            tile_size,
            overlap,
            ProcessingRecipe::None,
            requests,
        )
    }

    pub(crate) fn with_processed_requests(
        level: StableId,
        image_size: Vec2d,
        tile_size: Vec2d,
        overlap: Vec2d,
        processing: ProcessingRecipe,
        requests: impl Fn(GridTile) -> Request + Send + Sync + 'static,
    ) -> Result<Self, TileSourceError> {
        Self::new(
            level,
            image_size,
            tile_size,
            overlap,
            ClosureRequests {
                request: requests,
                processing,
            },
        )
    }

    #[must_use]
    pub const fn image_size(&self) -> Vec2d {
        self.image_size
    }

    #[must_use]
    pub const fn id(&self) -> &StableId {
        &self.level
    }

    #[must_use]
    pub const fn tile_size(&self) -> Vec2d {
        self.tile_size
    }

    #[must_use]
    pub const fn overlap(&self) -> Vec2d {
        self.overlap
    }

    #[must_use]
    pub const fn shape(&self) -> Vec2d {
        self.shape
    }

    #[must_use]
    pub const fn count(&self) -> u64 {
        self.count
    }

    /// Iterate tiles lazily in standardized row-major order.
    #[must_use]
    pub fn tiles_row_major(&self) -> GridTiles {
        GridTiles {
            grid: self.clone(),
            next: 0,
        }
    }

    fn grid_tile(&self, ordinal: u64) -> GridTile {
        let coord = GridCoord {
            column: u32::try_from(ordinal % u64::from(self.shape.x)).unwrap(),
            row: u32::try_from(ordinal / u64::from(self.shape.x)).unwrap(),
        };
        let origin = Vec2d {
            x: coord.column * self.tile_size.x,
            y: coord.row * self.tile_size.y,
        };
        let cell = self.tile_size.min(self.image_size - origin);
        let leading = Vec2d {
            x: self.overlap.x.min(origin.x),
            y: self.overlap.y.min(origin.y),
        };
        let remaining = self.image_size - origin - cell;
        let trailing = self.overlap.min(remaining);
        let destination = origin - leading;
        let expected_size = cell + leading + trailing;
        GridTile {
            coord,
            row_major_ordinal: ordinal,
            image_size: self.image_size,
            cell_size: self.tile_size,
            cell_extent: cell,
            destination,
            expected_size,
        }
    }

    fn tile(&self, ordinal: u64) -> TileSpec {
        let tile = self.grid_tile(ordinal);
        let mut request = self.requests.request(tile);
        if self.requests.use_first_tile_as_referer() {
            request
                .headers
                .entry("Referer".into())
                .or_insert_with(|| self.requests.request(self.grid_tile(0)).uri);
        }
        TileSpec {
            id: TileId::new(self.level.clone(), ordinal),
            request,
            destination: tile.destination,
            expected_size: Some(tile.expected_size),
            processing: self.requests.processing(),
            role: TileRole::Output,
        }
    }
}

pub struct GridTiles {
    grid: Grid,
    next: u64,
}

impl Iterator for GridTiles {
    type Item = Result<TileSpec, TileSourceError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.grid.count {
            return None;
        }
        let ordinal = self.next;
        self.next += 1;
        Some(Ok(self.grid.tile(ordinal)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = usize::try_from(self.grid.count - self.next).unwrap_or(usize::MAX);
        (remaining, Some(remaining))
    }
}

pub(crate) trait PositionedGenerator: fmt::Debug + Send + Sync {
    fn count(&self) -> u64;
    fn tile(&self, ordinal: u64) -> Result<PositionedTile, TileSourceError>;
}

#[derive(Clone, Debug)]
pub struct PositionedTile {
    pub request: Request,
    pub destination: Vec2d,
    pub processing: ProcessingRecipe,
}

#[derive(Debug)]
struct ExplicitPositioned(Arc<[PositionedTile]>);

impl PositionedGenerator for ExplicitPositioned {
    fn count(&self) -> u64 {
        self.0.len() as u64
    }

    fn tile(&self, ordinal: u64) -> Result<PositionedTile, TileSourceError> {
        usize::try_from(ordinal)
            .ok()
            .and_then(|index| self.0.get(index))
            .cloned()
            .ok_or_else(|| TileSourceError::InvalidTile("tile ordinal is out of bounds".into()))
    }
}

#[derive(Clone)]
pub struct Positioned {
    level: StableId,
    canvas_size: Option<Vec2d>,
    generator: Arc<dyn PositionedGenerator>,
}

impl fmt::Debug for Positioned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Positioned")
            .field("level", &self.level)
            .field("canvas_size", &self.canvas_size)
            .field("count", &self.count())
            .finish_non_exhaustive()
    }
}

impl Positioned {
    #[must_use]
    pub fn from_tiles(
        level: StableId,
        canvas_size: Option<Vec2d>,
        tiles: Vec<PositionedTile>,
    ) -> Self {
        Self::from_generator(level, canvas_size, ExplicitPositioned(tiles.into()))
    }

    pub(crate) fn from_generator(
        level: StableId,
        canvas_size: Option<Vec2d>,
        generator: impl PositionedGenerator + 'static,
    ) -> Self {
        Self {
            level,
            canvas_size,
            generator: Arc::new(generator),
        }
    }

    #[must_use]
    pub const fn image_size(&self) -> Option<Vec2d> {
        self.canvas_size
    }

    #[must_use]
    pub const fn id(&self) -> &StableId {
        &self.level
    }

    #[must_use]
    pub fn count(&self) -> u64 {
        self.generator.count()
    }

    #[must_use]
    pub fn tiles(&self) -> PositionedTiles {
        PositionedTiles {
            source: self.clone(),
            next: 0,
        }
    }
}

pub struct PositionedTiles {
    source: Positioned,
    next: u64,
}

impl Iterator for PositionedTiles {
    type Item = Result<TileSpec, TileSourceError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.source.count() {
            return None;
        }
        let ordinal = self.next;
        self.next += 1;
        Some(self.source.generator.tile(ordinal).map(|tile| TileSpec {
            id: TileId::new(self.source.level.clone(), ordinal),
            request: tile.request,
            destination: tile.destination,
            expected_size: None,
            processing: tile.processing,
            role: TileRole::Output,
        }))
    }
}

#[derive(Clone, Debug)]
pub enum TileSource {
    Grid(Grid),
    Positioned(Positioned),
    DiscoverableGrid(DiscoverableGrid),
    Adaptive(AdaptiveSource),
}

impl TileSource {
    #[must_use]
    pub fn id(&self) -> &StableId {
        match self {
            Self::Grid(grid) => grid.id(),
            Self::Positioned(positioned) => positioned.id(),
            Self::DiscoverableGrid(discoverable) => discoverable.id(),
            Self::Adaptive(adaptive) => adaptive.id(),
        }
    }

    #[must_use]
    pub fn image_size(&self) -> Option<Vec2d> {
        match self {
            Self::Grid(grid) => Some(grid.image_size()),
            Self::Positioned(positioned) => positioned.image_size(),
            Self::DiscoverableGrid(_) | Self::Adaptive(_) => None,
        }
    }

    #[must_use]
    pub fn tile_size(&self) -> Option<Vec2d> {
        match self {
            Self::Grid(grid) => Some(grid.tile_size()),
            Self::Positioned(_) | Self::DiscoverableGrid(_) | Self::Adaptive(_) => None,
        }
    }

    #[must_use]
    pub fn overlap(&self) -> Option<Vec2d> {
        match self {
            Self::Grid(grid) => Some(grid.overlap()),
            Self::Positioned(_) | Self::DiscoverableGrid(_) | Self::Adaptive(_) => None,
        }
    }

    #[must_use]
    pub fn count(&self) -> Option<u64> {
        match self {
            Self::Grid(grid) => Some(grid.count()),
            Self::Positioned(positioned) => Some(positioned.count()),
            Self::DiscoverableGrid(_) | Self::Adaptive(_) => None,
        }
    }
}

impl From<Grid> for TileSource {
    fn from(value: Grid) -> Self {
        Self::Grid(value)
    }
}

impl From<Positioned> for TileSource {
    fn from(value: Positioned) -> Self {
        Self::Positioned(value)
    }
}

impl From<DiscoverableGrid> for TileSource {
    fn from(value: DiscoverableGrid) -> Self {
        Self::DiscoverableGrid(value)
    }
}

impl From<AdaptiveSource> for TileSource {
    fn from(value: AdaptiveSource) -> Self {
        Self::Adaptive(value)
    }
}

impl fmt::Display for TileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.level, self.ordinal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Requests;

    impl GridRequests for Requests {
        fn request(&self, tile: GridTile) -> Request {
            Request::new(format!("memory://{}/{}", tile.coord.column, tile.coord.row))
        }
    }

    fn grid(overlap: Vec2d) -> Grid {
        Grid::new(
            "level".into(),
            Vec2d { x: 5, y: 4 },
            Vec2d { x: 3, y: 2 },
            overlap,
            Requests,
        )
        .unwrap()
    }

    #[test]
    fn exact_count_and_row_major_coordinates() {
        let grid = grid(Vec2d::default());
        assert_eq!(grid.shape(), Vec2d { x: 2, y: 2 });
        assert_eq!(grid.count(), 4);
        let tiles: Vec<_> = grid.tiles_row_major().collect::<Result<_, _>>().unwrap();
        assert_eq!(tiles[0].request.uri, "memory://0/0");
        assert_eq!(tiles[1].request.uri, "memory://1/0");
        assert_eq!(tiles[2].request.uri, "memory://0/1");
        assert_eq!(tiles[3].expected_size, Some(Vec2d { x: 2, y: 2 }));
    }

    #[test]
    fn overlap_rectangles_are_clipped_at_image_edges() {
        let tiles: Vec<_> = grid(Vec2d { x: 1, y: 1 })
            .tiles_row_major()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(tiles[0].destination, Vec2d { x: 0, y: 0 });
        assert_eq!(tiles[0].expected_size, Some(Vec2d { x: 4, y: 3 }));
        assert_eq!(tiles[3].destination, Vec2d { x: 2, y: 1 });
        assert_eq!(tiles[3].expected_size, Some(Vec2d { x: 3, y: 3 }));
    }

    #[test]
    fn zero_geometry_is_rejected() {
        assert_eq!(
            Grid::new(
                "level".into(),
                Vec2d { x: 0, y: 1 },
                Vec2d::square(1),
                Vec2d::default(),
                Requests,
            )
            .unwrap_err(),
            TileSourceError::ZeroImageDimensions
        );
        assert_eq!(
            Grid::new(
                "level".into(),
                Vec2d::square(1),
                Vec2d { x: 0, y: 1 },
                Vec2d::default(),
                Requests,
            )
            .unwrap_err(),
            TileSourceError::ZeroTileDimensions
        );
    }
}
