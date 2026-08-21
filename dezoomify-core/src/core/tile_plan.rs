//! Immutable, lazy and replayable known tile plans.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use crate::Vec2d;

use super::adaptive::{AdaptivePlan, TileObservation, TileProgram, TileProgramError};
use super::model::{ProcessingRecipe, Request, StableId, TileId, TileRole, TileSpec};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanError {
    ZeroCapacity,
    ArithmeticOverflow,
    InvalidTile(String),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => f.write_str("tile batch capacity must be greater than zero"),
            Self::ArithmeticOverflow => f.write_str("tile geometry overflowed u32"),
            Self::InvalidTile(message) => f.write_str(message),
        }
    }
}

impl Error for PlanError {}

/// Contract for a known plan: `tile(i)` is deterministic and returns `None`
/// exactly when `i >= len()`.
pub trait ReplayablePlan: fmt::Debug + Send + Sync {
    fn len(&self) -> u64;
    fn tile(&self, ordinal: u64) -> Result<Option<TileSpec>, PlanError>;

    /// Whether the plan contains no tiles.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone)]
pub struct KnownTilePlan(Arc<dyn ReplayablePlan>);

impl fmt::Debug for KnownTilePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KnownTilePlan")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl KnownTilePlan {
    #[must_use]
    pub(crate) fn new(plan: impl ReplayablePlan + 'static) -> Self {
        Self(Arc::new(plan))
    }

    #[must_use]
    pub fn rectangular(source: impl RectangularSource + 'static) -> Self {
        Self::new(RectangularPlan(source))
    }

    #[must_use]
    pub fn len(&self) -> u64 {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn tile(&self, ordinal: u64) -> Result<Option<TileSpec>, PlanError> {
        self.0.tile(ordinal)
    }

    #[must_use]
    pub fn cursor(&self) -> KnownPlanCursor {
        KnownPlanCursor {
            plan: self.clone(),
            next: 0,
        }
    }
}

#[derive(Clone)]
pub enum LevelPlan {
    Known(KnownTilePlan),
    Adaptive(Arc<dyn AdaptivePlan>),
}

impl LevelPlan {
    #[must_use]
    pub fn start_program(&self) -> Box<dyn TileProgram> {
        match self {
            Self::Known(plan) => Box::new(plan.cursor()),
            Self::Adaptive(plan) => plan.start(),
        }
    }
}

impl fmt::Debug for LevelPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Known(plan) => fmt::Debug::fmt(plan, f),
            Self::Adaptive(plan) => f.debug_tuple("Adaptive").field(plan).finish(),
        }
    }
}

/// Common rectangular protocol surface. Format structs remain immutable and
/// supply only geometry, request generation, and optional overlap/processing.
pub trait RectangularSource: fmt::Debug + Send + Sync {
    fn level_id(&self) -> StableId;
    fn image_size(&self) -> Vec2d;
    fn tile_size(&self) -> Vec2d;
    fn request(&self, cell: Vec2d) -> Request;

    fn overlap(&self) -> Vec2d {
        Vec2d::default()
    }

    fn processing(&self) -> ProcessingRecipe {
        ProcessingRecipe::None
    }
}

#[derive(Debug)]
struct RectangularPlan<S>(S);

impl<S: RectangularSource> ReplayablePlan for RectangularPlan<S> {
    fn len(&self) -> u64 {
        let size = self.0.image_size();
        let tile = self.0.tile_size();
        if tile.x == 0 || tile.y == 0 {
            return 0;
        }
        u64::from(size.x.div_ceil(tile.x)) * u64::from(size.y.div_ceil(tile.y))
    }

    fn tile(&self, ordinal: u64) -> Result<Option<TileSpec>, PlanError> {
        if ordinal >= self.len() {
            return Ok(None);
        }
        let image = self.0.image_size();
        let tile = self.0.tile_size();
        let columns = image.x.div_ceil(tile.x);
        let x = u32::try_from(ordinal % u64::from(columns))
            .map_err(|_| PlanError::ArithmeticOverflow)?;
        let y = u32::try_from(ordinal / u64::from(columns))
            .map_err(|_| PlanError::ArithmeticOverflow)?;
        let cell = Vec2d { x, y };
        let origin = Vec2d {
            x: x.checked_mul(tile.x).ok_or(PlanError::ArithmeticOverflow)?,
            y: y.checked_mul(tile.y).ok_or(PlanError::ArithmeticOverflow)?,
        };
        let clipped = Vec2d {
            x: tile.x.min(image.x.saturating_sub(origin.x)),
            y: tile.y.min(image.y.saturating_sub(origin.y)),
        };
        let overlap = self.0.overlap();
        let leading = Vec2d {
            x: if x == 0 { 0 } else { overlap.x },
            y: if y == 0 { 0 } else { overlap.y },
        };
        let destination = Vec2d {
            x: origin.x.saturating_sub(leading.x),
            y: origin.y.saturating_sub(leading.y),
        };
        let mut request = self.0.request(cell);
        request
            .headers
            .entry("Referer".into())
            .or_insert_with(|| self.0.request(Vec2d::default()).uri);
        Ok(Some(TileSpec {
            id: TileId::new(self.0.level_id(), ordinal),
            request,
            destination,
            expected_size: (overlap == Vec2d::default()).then_some(clipped),
            processing: self.0.processing(),
            role: TileRole::Output,
        }))
    }
}

#[derive(Clone, Debug)]
pub struct KnownPlanCursor {
    plan: KnownTilePlan,
    next: u64,
}

impl KnownPlanCursor {
    fn take_ready_inner(&mut self, capacity: usize) -> Result<Option<Vec<TileSpec>>, TileProgramError> {
        if capacity == 0 {
            return Err(TileProgramError::ZeroCapacity);
        }
        let remaining = self.plan.len().saturating_sub(self.next);
        let remaining = usize::try_from(remaining).unwrap_or(usize::MAX);
        let mut batch = Vec::with_capacity(capacity.min(remaining));
        while batch.len() < capacity {
            let Some(spec) = self
                .plan
                .0
                .tile(self.next)
                .map_err(|error| match error {
                    PlanError::ZeroCapacity => TileProgramError::ZeroCapacity,
                    PlanError::ArithmeticOverflow => TileProgramError::ArithmeticOverflow,
                    PlanError::InvalidTile(_) => {
                        TileProgramError::ArithmeticOverflow
                    }
                })?
            else {
                break;
            };
            self.next = self
                .next
                .checked_add(1)
                .ok_or(TileProgramError::ArithmeticOverflow)?;
            batch.push(spec);
        }
        Ok((!batch.is_empty()).then_some(batch))
    }

    /// Convenience for callers not going through the `TileProgram` trait.
    pub fn take_ready(&mut self, capacity: usize) -> Result<Option<Vec<TileSpec>>, TileProgramError> {
        self.take_ready_inner(capacity)
    }

}

impl TileProgram for KnownPlanCursor {
    fn take_ready(&mut self, capacity: usize) -> Result<Option<Vec<TileSpec>>, TileProgramError> {
        self.take_ready_inner(capacity)
    }

    fn submit(&mut self, _observations: &[TileObservation]) -> Result<(), TileProgramError> {
        Ok(())
    }

    fn image_size(&self) -> Option<Vec2d> {
        None
    }
}

impl fmt::Display for TileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.level, self.ordinal)
    }
}

#[cfg(test)]
mod tests {
    use super::super::adaptive::TileProgramError;
    use super::*;

    #[derive(Debug)]
    struct Grid {
        overlap: Vec2d,
    }

    impl RectangularSource for Grid {
        fn level_id(&self) -> StableId {
            "level".into()
        }
        fn image_size(&self) -> Vec2d {
            Vec2d { x: 5, y: 4 }
        }
        fn tile_size(&self) -> Vec2d {
            Vec2d { x: 3, y: 2 }
        }
        fn request(&self, p: Vec2d) -> Request {
            Request::new(format!("memory://{}/{}", p.x, p.y))
        }
        fn overlap(&self) -> Vec2d {
            self.overlap
        }
    }

    fn plan(overlap: Vec2d) -> KnownTilePlan {
        KnownTilePlan::rectangular(Grid { overlap })
    }

    fn spec(uri: &str, ordinal: u64) -> TileSpec {
        TileSpec {
            id: TileId::new("level".into(), ordinal),
            request: Request::new(uri),
            destination: Vec2d::default(),
            expected_size: Some(Vec2d { x: 1, y: 1 }),
            processing: ProcessingRecipe::None,
            role: TileRole::Output,
        }
    }

    #[test]
    fn rectangular_plan_is_lazy_row_major_and_replayable() {
        let plan = plan(Vec2d::default());
        assert_eq!(plan.len(), 4);
        let collect = || {
            let mut cursor = plan.cursor();
            cursor.take_ready(10).unwrap().unwrap()
        };
        let first = collect();
        assert_eq!(first, collect());
        assert_eq!(first[3].request.uri, "memory://1/1");
        assert_eq!(first[3].expected_size, Some(Vec2d { x: 2, y: 2 }));
        assert_eq!(
            first[3].request.headers.get("Referer").map(String::as_str),
            Some("memory://0/0")
        );
    }

    #[test]
    fn cursor_batches_without_mutating_plan() {
        let plan = plan(Vec2d::default());
        let mut cursor = plan.cursor();
        assert_eq!(cursor.take_ready(3).unwrap().unwrap().len(), 3);
        assert_eq!(cursor.take_ready(3).unwrap().unwrap().len(), 1);
        assert_eq!(cursor.take_ready(3).unwrap(), None);
        // After consuming all tiles, further calls return None
        assert_eq!(plan.len(), 4);
    }

    #[test]
    fn zero_capacity_is_typed_error() {
        assert_eq!(
            plan(Vec2d::default()).cursor().take_ready(0),
            Err(TileProgramError::ZeroCapacity)
        );
    }

    #[test]
    fn overlap_is_applied_only_after_leading_edges() {
        let plan = plan(Vec2d { x: 1, y: 1 });
        let origins: Vec<_> = (0..plan.len())
            .map(|i| plan.tile(i).unwrap().unwrap().destination)
            .collect();
        assert_eq!(
            origins,
            [
                Vec2d { x: 0, y: 0 },
                Vec2d { x: 2, y: 0 },
                Vec2d { x: 0, y: 1 },
                Vec2d { x: 2, y: 1 }
            ]
        );
    }
}
