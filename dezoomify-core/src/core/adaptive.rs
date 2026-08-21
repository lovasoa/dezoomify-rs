//! Observation-driven plans, including the single Generic implementation.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::Vec2d;

use super::model::{ProcessingRecipe, Request, StableId, TileId, TileRole, TileSpec};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TileObservation {
    pub id: TileId,
    pub result: ObservationResult,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationResult {
    Available { size: Vec2d },
    Missing,
}

impl TileObservation {
    #[must_use]
    pub fn available(id: TileId, size: Vec2d) -> Self {
        Self {
            id,
            result: ObservationResult::Available { size },
        }
    }

    #[must_use]
    pub fn missing(id: TileId) -> Self {
        Self {
            id,
            result: ObservationResult::Missing,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TileProgramError {
    ZeroCapacity,
    PendingObservation,
    NoPendingObservation,
    InvalidObservationCount(usize),
    InvalidObservation(TileId),
    InvalidDimensions(TileId),
    ArithmeticOverflow,
}

impl fmt::Display for TileProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => f.write_str("tile batch capacity must be greater than zero"),
            Self::PendingObservation => f.write_str("a probe observation is pending"),
            Self::NoPendingObservation => f.write_str("no probe observation is pending"),
            Self::InvalidObservationCount(count) => {
                write!(f, "expected one probe observation, received {count}")
            }
            Self::InvalidObservation(id) => write!(f, "unexpected observation for {id}"),
            Self::InvalidDimensions(id) => write!(f, "invalid dimensions for {id}"),
            Self::ArithmeticOverflow => f.write_str("tile geometry overflowed u32"),
        }
    }
}

impl Error for TileProgramError {}

/// Backwards-compatible alias — the unified batch error was historically named `AdaptiveError`.
pub type AdaptiveError = TileProgramError;

pub trait AdaptivePlan: fmt::Debug + Send + Sync {
    fn start(&self) -> Box<dyn TileProgram>;
}

pub trait TileProgram: fmt::Debug + Send {
    fn take_ready(&mut self, capacity: usize) -> Result<Option<Vec<TileSpec>>, TileProgramError>;
    fn submit(&mut self, observations: &[TileObservation]) -> Result<(), TileProgramError>;
    fn image_size(&self) -> Option<Vec2d>;
}

static TEMPLATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?xi)\{\{\s*(?P<dimension>x|y)(?::0(?P<zeroes>\d+))?\s*\}\}")
        .expect("constant generic template pattern")
});

/// Returns true if the template contains at least one `{{x}}`/`{{y}}` placeholder.
///
/// The grammar is `{{x}}`, `{{y}}` (case-insensitive, optional whitespace) with an
/// optional zero-padded width `{{x:02}}` / `{{y:05}}` requiring a leading `0`.
#[must_use]
pub fn is_generic_template(template: &str) -> bool {
    TEMPLATE_RE.is_match(template)
}

pub(crate) fn render_template(template: &str, x: u32, y: u32) -> String {
    TEMPLATE_RE
        .replace_all(template, |caps: &Captures| {
            let dimension = caps
                .name("dimension")
                .expect("missing dimension")
                .as_str()
                .to_ascii_lowercase();
            let num = match dimension.as_str() {
                "x" => x,
                "y" => y,
                _ => unreachable!("dimension is x or y"),
            };
            let padding: usize = caps
                .name("zeroes")
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            format!("{num:0padding$}")
        })
        .into_owned()
}

/// Immutable descriptor for the Generic URL-template protocol.
#[derive(Clone, Debug)]
pub struct GenericAdaptivePlan {
    pub level: StableId,
    pub template: String,
}

impl GenericAdaptivePlan {
    #[must_use]
    pub fn new(level: impl Into<StableId>, template: impl Into<String>) -> Self {
        Self {
            level: level.into(),
            template: template.into(),
        }
    }
}

impl AdaptivePlan for GenericAdaptivePlan {
    fn start(&self) -> Box<dyn TileProgram> {
        Box::new(GenericProgram::new(self.clone()))
    }
}

#[derive(Debug)]
struct GenericProgram {
    plan: GenericAdaptivePlan,
    search: Dichotomy2d,
    pending: Option<(TileId, Vec2d)>,
    observed: HashMap<Vec2d, bool>,
    next_point: Vec2d,
    next_id: u64,
    tile_size: Option<Vec2d>,
    image_size: Option<Vec2d>,
    grid_size: Option<Vec2d>,
    output_cursor: u64,
}

impl GenericProgram {
    fn new(plan: GenericAdaptivePlan) -> Self {
        Self {
            plan,
            search: Dichotomy2d::default(),
            pending: None,
            observed: HashMap::new(),
            next_point: Vec2d::default(),
            next_id: 0,
            tile_size: None,
            image_size: None,
            grid_size: None,
            output_cursor: 0,
        }
    }

    fn spec(&mut self, point: Vec2d, role: TileRole) -> Result<TileSpec, TileProgramError> {
        let size = self.tile_size.unwrap_or_default();
        let origin = Vec2d {
            x: point
                .x
                .checked_mul(size.x)
                .ok_or(TileProgramError::ArithmeticOverflow)?,
            y: point
                .y
                .checked_mul(size.y)
                .ok_or(TileProgramError::ArithmeticOverflow)?,
        };
        let id = TileId::new(self.plan.level.clone(), self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(TileProgramError::ArithmeticOverflow)?;
        Ok(TileSpec {
            id,
            request: Request::new(render_template(&self.plan.template, point.x, point.y)),
            source_region: None,
            destination: origin,
            expected_size: (role == TileRole::Output).then_some(size),
            processing: ProcessingRecipe::None,
            role,
        })
    }

    fn finish_search(&mut self, last: Vec2d) -> Result<(), TileProgramError> {
        let Some(tile) = self.tile_size else {
            self.grid_size = Some(Vec2d::default());
            return Ok(());
        };
        let grid = Vec2d {
            x: last
                .x
                .checked_add(1)
                .ok_or(TileProgramError::ArithmeticOverflow)?,
            y: last
                .y
                .checked_add(1)
                .ok_or(TileProgramError::ArithmeticOverflow)?,
        };
        self.image_size = Some(Vec2d {
            x: grid
                .x
                .checked_mul(tile.x)
                .ok_or(TileProgramError::ArithmeticOverflow)?,
            y: grid
                .y
                .checked_mul(tile.y)
                .ok_or(TileProgramError::ArithmeticOverflow)?,
        });
        self.grid_size = Some(grid);
        Ok(())
    }
}

impl TileProgram for GenericProgram {
    fn take_ready(&mut self, capacity: usize) -> Result<Option<Vec<TileSpec>>, TileProgramError> {
        if capacity == 0 {
            return Err(TileProgramError::ZeroCapacity);
        }
        if self.pending.is_some() {
            return Err(TileProgramError::PendingObservation);
        }
        if let Some(grid) = self.grid_size {
            let total = u64::from(grid.x) * u64::from(grid.y);
            let mut batch = Vec::with_capacity(capacity);
            while self.output_cursor < total && batch.len() < capacity {
                let ordinal = self.output_cursor;
                self.output_cursor += 1;
                let point = Vec2d {
                    x: u32::try_from(ordinal % u64::from(grid.x))
                        .map_err(|_| TileProgramError::ArithmeticOverflow)?,
                    y: u32::try_from(ordinal / u64::from(grid.x))
                        .map_err(|_| TileProgramError::ArithmeticOverflow)?,
                };
                if !self.observed.contains_key(&point) {
                    batch.push(self.spec(point, TileRole::Output)?);
                }
            }
            return Ok((!batch.is_empty()).then_some(batch));
        }

        let point = self.next_point;
        let spec = self.spec(point, TileRole::ProbeAndOutput)?;
        self.pending = Some((spec.id.clone(), point));
        Ok(Some(vec![spec]))
    }

    fn submit(&mut self, observations: &[TileObservation]) -> Result<(), TileProgramError> {
        let Some((expected, point)) = self.pending.take() else {
            return Err(TileProgramError::NoPendingObservation);
        };
        if observations.len() != 1 {
            self.pending = Some((expected.clone(), point));
            return Err(TileProgramError::InvalidObservationCount(observations.len()));
        }
        if observations[0].id != expected {
            self.pending = Some((expected.clone(), point));
            return Err(TileProgramError::InvalidObservation(
                observations[0].id.clone(),
            ));
        }
        let success = match observations[0].result {
            ObservationResult::Available { size } if size.x > 0 && size.y > 0 => {
                self.tile_size.get_or_insert(size);
                true
            }
            ObservationResult::Available { .. } => {
                self.pending = Some((expected.clone(), point));
                return Err(TileProgramError::InvalidDimensions(expected));
            }
            ObservationResult::Missing => false,
        };
        let first = self.observed.is_empty();
        self.observed.insert(point, success);
        if first {
            if success {
                self.next_point = Dichotomy2d::first();
            } else {
                self.grid_size = Some(Vec2d::default());
            }
            return Ok(());
        }
        let mut next = self.search.next(success);
        while let Some(point) = next {
            let Some(previous) = self.observed.get(&point) else {
                self.next_point = point;
                return Ok(());
            };
            next = self.search.next(*previous);
        }
        let boundary = self.search.boundary();
        self.finish_search(boundary)
    }

    fn image_size(&self) -> Option<Vec2d> {
        self.image_size
    }
}

#[derive(Debug, Default)]
struct Dichotomy {
    min: u32,
    max: Option<u32>,
}

impl Dichotomy {
    fn guess(&self) -> u32 {
        self.max.map_or_else(
            || self.min.saturating_mul(3).saturating_add(1),
            |max| u32::midpoint(max, self.min),
        )
    }

    fn next(&mut self, success: bool) -> Option<u32> {
        let previous = self.guess();
        if success {
            self.min = previous;
        } else {
            self.max = Some(previous);
        }
        let next = self.guess();
        (next != previous).then_some(next)
    }
}

#[derive(Debug)]
enum Dichotomy2d {
    Diagonal(Dichotomy),
    Orientation(u32),
    Last {
        diagonal: u32,
        landscape: bool,
        dimension: Dichotomy,
    },
}

impl Default for Dichotomy2d {
    fn default() -> Self {
        Self::Diagonal(Dichotomy::default())
    }
}

impl Dichotomy2d {
    const fn first() -> Vec2d {
        Vec2d { x: 1, y: 1 }
    }

    fn boundary(&self) -> Vec2d {
        match self {
            Self::Diagonal(search) => Vec2d {
                x: search.guess(),
                y: search.guess(),
            },
            Self::Orientation(diagonal) => Vec2d {
                x: *diagonal,
                y: *diagonal,
            },
            Self::Last {
                diagonal,
                landscape,
                dimension,
            } => {
                let last = dimension.guess();
                if *landscape {
                    Vec2d {
                        x: last,
                        y: *diagonal,
                    }
                } else {
                    Vec2d {
                        x: *diagonal,
                        y: last,
                    }
                }
            }
        }
    }

    fn next(&mut self, success: bool) -> Option<Vec2d> {
        let mut transition = None;
        let result = match self {
            Self::Diagonal(search) => search.next(success).map_or_else(
                || {
                    let diagonal = search.guess();
                    transition = Some(Self::Orientation(diagonal));
                    Some(Vec2d {
                        x: diagonal + 1,
                        y: diagonal,
                    })
                },
                |next| Some(Vec2d { x: next, y: next }),
            ),
            Self::Orientation(diagonal) => {
                let search = Dichotomy {
                    min: *diagonal + u32::from(success),
                    max: None,
                };
                let next = search.guess();
                transition = Some(Self::Last {
                    diagonal: *diagonal,
                    landscape: success,
                    dimension: search,
                });
                Some(if success {
                    Vec2d {
                        x: next,
                        y: *diagonal,
                    }
                } else {
                    Vec2d {
                        x: *diagonal,
                        y: next,
                    }
                })
            }
            Self::Last {
                diagonal,
                landscape,
                dimension,
            } => dimension.next(success).map(|next| {
                if *landscape {
                    Vec2d {
                        x: next,
                        y: *diagonal,
                    }
                } else {
                    Vec2d {
                        x: *diagonal,
                        y: next,
                    }
                }
            }),
        };
        if let Some(state) = transition {
            *self = state;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn generic(level: &str) -> Box<dyn TileProgram> {
        GenericAdaptivePlan::new(level, "memory://{{X:02}},{{y}}").start()
    }

    #[test]
    fn generic_is_bounded_and_probe_misses_are_not_output() {
        let plan = GenericAdaptivePlan::new("level", "{{X}},{{Y}}");
        let mut program = plan.start();
        let existing = ["0,0", "1,0", "2,0", "0,1", "1,1", "2,1"];
        let mut available = Vec::new();
        for _ in 0..20 {
            let Some(batch) = program.take_ready(2).unwrap() else {
                break;
            };
            if batch[0].role == TileRole::Output {
                assert!(batch.len() <= 2);
                available.extend(batch);
                continue;
            }
            let spec = &batch[0];
            if existing.contains(&spec.request.uri.as_str()) {
                available.push(spec.clone());
                program
                    .submit(&[TileObservation::available(
                        spec.id.clone(),
                        Vec2d { x: 4, y: 5 },
                    )])
                    .unwrap();
            } else {
                program
                    .submit(&[TileObservation::missing(spec.id.clone())])
                    .unwrap();
            }
        }
        assert_eq!(program.image_size(), Some(Vec2d { x: 12, y: 10 }));
        assert_eq!(available.len(), 6);
        assert!(
            available
                .iter()
                .any(|tile| tile.role == TileRole::ProbeAndOutput)
        );
        assert!(
            available
                .iter()
                .all(|tile| existing.contains(&tile.request.uri.as_str()))
        );
        assert_eq!(
            available
                .iter()
                .map(|tile| &tile.id)
                .collect::<HashSet<_>>()
                .len(),
            available.len()
        );
        assert_eq!(
            available
                .iter()
                .find(|tile| tile.request.uri == "1,1")
                .unwrap()
                .destination,
            Vec2d { x: 4, y: 5 }
        );
    }

    #[test]
    fn sequencing_errors_preserve_the_pending_probe() {
        let mut program = generic("level");
        assert_eq!(
            program.submit(&[]),
            Err(AdaptiveError::NoPendingObservation)
        );
        assert_eq!(program.take_ready(0), Err(AdaptiveError::ZeroCapacity));
        let probe = program.take_ready(1).unwrap().unwrap().remove(0);
        assert_eq!(probe.request.uri, "memory://00,0");
        assert_eq!(
            program.take_ready(1),
            Err(AdaptiveError::PendingObservation)
        );
        assert_eq!(
            program.submit(&[]),
            Err(AdaptiveError::InvalidObservationCount(0))
        );
        assert_eq!(
            program.submit(&[
                TileObservation::missing(probe.id.clone()),
                TileObservation::missing(probe.id.clone()),
            ]),
            Err(AdaptiveError::InvalidObservationCount(2))
        );
        let unknown = TileId::new("other".into(), 0);
        assert_eq!(
            program.submit(&[TileObservation::missing(unknown.clone())]),
            Err(AdaptiveError::InvalidObservation(unknown))
        );
        assert_eq!(
            program.submit(&[TileObservation::available(
                probe.id.clone(),
                Vec2d::default()
            )]),
            Err(AdaptiveError::InvalidDimensions(probe.id.clone()))
        );
        program
            .submit(&[TileObservation::missing(probe.id)])
            .unwrap();
        assert_eq!(program.take_ready(1).unwrap(), None);
    }

    #[test]
    fn programs_do_not_share_probe_state() {
        let mut first = generic("first");
        let mut second = generic("second");
        let first_probe = first.take_ready(1).unwrap().unwrap().remove(0);
        let second_probe = second.take_ready(1).unwrap().unwrap().remove(0);
        assert_ne!(first_probe.id, second_probe.id);
        first
            .submit(&[TileObservation::missing(first_probe.id)])
            .unwrap();
        assert_eq!(second.take_ready(1), Err(AdaptiveError::PendingObservation));
    }

    #[test]
    fn generic_templates_are_case_insensitive_and_zero_padded() {
        let mut program = GenericAdaptivePlan::new(
            "level",
            "https://example.test/{{x:05}}/{{Y:03}}/{{unknown}}",
        )
        .start();
        let probe = program.take_ready(1).unwrap().unwrap().remove(0);
        assert_eq!(
            probe.request.uri,
            "https://example.test/00000/000/{{unknown}}"
        );
        program
            .submit(&[TileObservation::available(probe.id, Vec2d { x: 4, y: 5 })])
            .unwrap();
        let next = program.take_ready(1).unwrap().unwrap().remove(0);
        assert_eq!(
            next.request.uri,
            "https://example.test/00001/001/{{unknown}}"
        );
        assert_eq!(next.role, TileRole::ProbeAndOutput);
    }

    #[test]
    fn dichotomies_find_exact_geometry() {
        for boundary in 0..1000 {
            let mut search = Dichotomy::default();
            let mut guess = search.guess();
            for _ in 0..20 {
                let Some(next) = search.next(guess <= boundary) else {
                    break;
                };
                guess = next;
            }
            assert_eq!(search.guess(), boundary);
        }
        for x in 0..10 {
            for y in 0..10 {
                let mut search = Dichotomy2d::default();
                let mut guess = Dichotomy2d::first();
                for _ in 0..20 {
                    let Some(next) = search.next(guess.x <= x && guess.y <= y) else {
                        break;
                    };
                    guess = next;
                }
                assert_eq!(search.boundary(), Vec2d { x, y });
            }
        }
    }
}
