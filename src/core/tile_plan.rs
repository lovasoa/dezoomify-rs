use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use super::model::{
    Dimensions, Point, ProcessingRecipe, Region, RequestSpec, StableId, TileId, TileRole, TileSpec,
};

/// Errors returned by pure tile-plan construction and iteration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanError {
    ZeroCapacity,
    DuplicateTileId(TileId),
    ArithmeticOverflow,
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => f.write_str("tile-plan batch capacity must be greater than zero"),
            Self::DuplicateTileId(id) => write!(f, "duplicate tile id: {id:?}"),
            Self::ArithmeticOverflow => f.write_str("tile-plan geometry overflowed u32"),
        }
    }
}

impl Error for PlanError {}

/// An immutable, replayable ordered collection of tile specifications.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnownTilePlan {
    specs: Arc<[TileSpec]>,
}

impl KnownTilePlan {
    /// Validates IDs and freezes the order of the supplied specifications.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::DuplicateTileId`] when two specifications have
    /// the same identity.
    pub fn new(specs: impl IntoIterator<Item = TileSpec>) -> Result<Self, PlanError> {
        let specs: Vec<_> = specs.into_iter().collect();
        let mut ids = HashSet::with_capacity(specs.len());
        for spec in &specs {
            if !ids.insert(spec.id.clone()) {
                return Err(PlanError::DuplicateTileId(spec.id.clone()));
            }
        }
        Ok(Self {
            specs: specs.into(),
        })
    }

    #[must_use]
    pub fn specs(&self) -> &[TileSpec] {
        &self.specs
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    #[must_use]
    pub fn cursor(&self) -> KnownPlanCursor {
        KnownPlanCursor {
            plan: self.clone(),
            next: 0,
        }
    }

    /// Creates a canonical row-major rectangular grid.
    ///
    /// `overlap` is applied to the destination origin for every non-leading
    /// row or column.  The source and destination regions describe the actual
    /// clipped image rectangle, which keeps edge geometry deterministic while
    /// allowing the application to decode larger protocol tiles when needed.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::ArithmeticOverflow`] if grid geometry cannot be
    /// represented by the pure `u32` geometry types.
    pub fn rectangular_grid<F>(
        level: impl Into<StableId>,
        image_size: Dimensions,
        tile_size: Dimensions,
        overlap: Point,
        mut request_for: F,
    ) -> Result<Self, PlanError>
    where
        F: FnMut(Point) -> RequestSpec,
    {
        let level = level.into();
        if tile_size.is_empty() {
            return Ok(Self {
                specs: Arc::from([]),
            });
        }

        let columns = image_size.width.div_ceil(tile_size.width);
        let rows = image_size.height.div_ceil(tile_size.height);
        let mut specs = Vec::with_capacity((columns as usize).saturating_mul(rows as usize));

        for y in 0..rows {
            for x in 0..columns {
                let coordinate = Point::new(x, y);
                let image_origin = Point::new(
                    x.checked_mul(tile_size.width)
                        .ok_or(PlanError::ArithmeticOverflow)?,
                    y.checked_mul(tile_size.height)
                        .ok_or(PlanError::ArithmeticOverflow)?,
                );
                let clipped_size = Dimensions::new(
                    tile_size
                        .width
                        .min(image_size.width.saturating_sub(image_origin.x)),
                    tile_size
                        .height
                        .min(image_size.height.saturating_sub(image_origin.y)),
                );
                let applied_overlap = Point::new(
                    if x == 0 { 0 } else { overlap.x },
                    if y == 0 { 0 } else { overlap.y },
                );
                let destination_origin = image_origin.saturating_sub(applied_overlap);
                let destination_size = Dimensions::new(
                    clipped_size
                        .width
                        .checked_add(applied_overlap.x)
                        .ok_or(PlanError::ArithmeticOverflow)?,
                    clipped_size
                        .height
                        .checked_add(applied_overlap.y)
                        .ok_or(PlanError::ArithmeticOverflow)?,
                );
                let region = Region::new(destination_origin, destination_size);
                specs.push(TileSpec {
                    id: TileId::new(level.clone(), TileRole::Output, specs.len() as u64),
                    request: request_for(coordinate),
                    source_region: region,
                    destination_region: region,
                    expected_size: Some(destination_size),
                    processing: ProcessingRecipe::None,
                    role: TileRole::Output,
                });
            }
        }
        Self::new(specs)
    }
}

/// Stateful cursor over an immutable known plan.
#[derive(Clone, Debug)]
pub struct KnownPlanCursor {
    plan: KnownTilePlan,
    next: usize,
}

impl KnownPlanCursor {
    /// Returns at most `capacity` specs in canonical order.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::ZeroCapacity`] when `capacity` is zero.
    pub fn take_ready(&mut self, capacity: usize) -> Result<Option<Vec<TileSpec>>, PlanError> {
        if capacity == 0 {
            return Err(PlanError::ZeroCapacity);
        }
        if self.next == self.plan.len() {
            return Ok(None);
        }
        let end = self.next.saturating_add(capacity).min(self.plan.len());
        let result = self.plan.specs[self.next..end].to_vec();
        self.next = end;
        Ok(Some(result))
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.next == self.plan.len()
    }

    #[must_use]
    pub fn remaining(&self) -> usize {
        self.plan.len().saturating_sub(self.next)
    }
}

impl fmt::Display for TileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{:?}:{}", self.level, self.role, self.ordinal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(point: Point) -> RequestSpec {
        RequestSpec::new(format!("memory://{}/{}", point.x, point.y))
    }

    #[test]
    fn rectangular_grid_is_row_major_and_replayable() {
        let first = KnownTilePlan::rectangular_grid(
            StableId::from("level-1"),
            Dimensions::new(5, 4),
            Dimensions::new(3, 2),
            Point::default(),
            request,
        )
        .unwrap();
        let second = KnownTilePlan::rectangular_grid(
            StableId::from("level-1"),
            Dimensions::new(5, 4),
            Dimensions::new(3, 2),
            Point::default(),
            request,
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(
            first
                .specs()
                .iter()
                .map(|spec| spec.request.uri.as_str())
                .collect::<Vec<_>>(),
            [
                "memory://0/0",
                "memory://1/0",
                "memory://0/1",
                "memory://1/1"
            ]
        );
    }

    #[test]
    fn cursor_batches_without_mutating_the_plan() {
        let plan = KnownTilePlan::rectangular_grid(
            StableId::from("level"),
            Dimensions::new(5, 4),
            Dimensions::new(3, 2),
            Point::default(),
            request,
        )
        .unwrap();
        let mut cursor = plan.cursor();
        assert_eq!(cursor.take_ready(3).unwrap().unwrap().len(), 3);
        assert_eq!(cursor.take_ready(3).unwrap().unwrap().len(), 1);
        assert_eq!(cursor.take_ready(3).unwrap(), None);
        assert!(cursor.is_finished());
        assert_eq!(plan.len(), 4);
        assert_eq!(cursor.remaining(), 0);
    }

    #[test]
    fn zero_capacity_is_typed_error() {
        let plan = KnownTilePlan::new([]).unwrap();
        assert_eq!(plan.cursor().take_ready(0), Err(PlanError::ZeroCapacity));
    }

    #[test]
    fn overlap_is_applied_only_after_leading_edges() {
        let plan = KnownTilePlan::rectangular_grid(
            StableId::from("level"),
            Dimensions::new(5, 4),
            Dimensions::new(3, 3),
            Point::new(1, 2),
            request,
        )
        .unwrap();
        assert_eq!(plan.specs()[0].destination_region.origin, Point::new(0, 0));
        assert_eq!(plan.specs()[1].destination_region.origin, Point::new(2, 0));
        assert_eq!(plan.specs()[2].destination_region.origin, Point::new(0, 1));
        assert_eq!(plan.specs()[3].destination_region.origin, Point::new(2, 1));
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let id = TileId::new(StableId::from("level"), TileRole::Output, 0);
        let spec = TileSpec {
            id: id.clone(),
            request: RequestSpec::new("memory://one"),
            source_region: Region::new(Point::default(), Dimensions::new(1, 1)),
            destination_region: Region::new(Point::default(), Dimensions::new(1, 1)),
            expected_size: Some(Dimensions::new(1, 1)),
            processing: ProcessingRecipe::None,
            role: TileRole::Output,
        };
        let duplicate = TileSpec {
            request: RequestSpec::new("memory://two"),
            ..spec.clone()
        };
        assert_eq!(
            KnownTilePlan::new([spec, duplicate]),
            Err(PlanError::DuplicateTileId(id))
        );
    }
}
