//! Resolution of generic origin-anchored solid tile rectangles.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock};

use regex::Regex;

use crate::Vec2d;
use crate::template::{Part, Template};

use super::model::{Request, StableId, TileId, TileRole, TileSpec};
use super::tile_plan::{Grid, GridRequests, GridTile, TileSourceError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationResult {
    Available { size: Vec2d },
    Missing,
}

static TEMPLATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?xi)(?:
            \{\{\s*(?P<dimension>x|y)(?::0(?P<zeroes>\d+))?\s*\}\}
            |
            %7b%7b\s*(?P<encoded_dimension>x|y)(?:(?:%3a|:)[^0-9]?(?P<encoded_zeroes>\d+))?\s*%7d%7d
        )",
    )
    .expect("constant generic template pattern")
});

#[must_use]
pub fn is_generic_template(template: &str) -> bool {
    TEMPLATE_RE.is_match(template)
}

#[derive(Clone, Copy, Debug)]
enum Dimension {
    X,
    Y,
}

fn parse_template(input: String) -> Template<Dimension> {
    let input = input.into_boxed_str();
    let mut cursor = 0;
    let mut parts = Vec::new();
    for captures in TEMPLATE_RE.captures_iter(&input) {
        let matched = captures.get(0).expect("a capture has a full match");
        parts.push(Part::literal(&input[cursor..matched.start()]));
        let dimension_name = captures
            .name("dimension")
            .or_else(|| captures.name("encoded_dimension"))
            .expect("a generic template has a dimension")
            .as_str();
        let dimension = if dimension_name.eq_ignore_ascii_case("x") {
            Dimension::X
        } else {
            Dimension::Y
        };
        let padding = captures
            .name("zeroes")
            .or_else(|| captures.name("encoded_zeroes"))
            .and_then(|value| value.as_str().parse().ok())
            .unwrap_or(0);
        parts.push(Part::Hole(dimension, padding));
        cursor = matched.end();
    }
    parts.push(Part::literal(&input[cursor..]));
    Template(parts)
}

fn render_template(template: &Template<Dimension>, x: u32, y: u32) -> String {
    template.render(|dimension| match dimension {
        Dimension::X => x,
        Dimension::Y => y,
    })
}

#[derive(Clone, Debug)]
pub struct DiscoverableGrid {
    level: StableId,
    template: Template<Dimension>,
}

/// A probe-driven source whose first tile determines a regular output grid.
///
/// The source owns only the pure program which creates the first probe. The
/// host supplies the decoded tile dimensions through [`ProbeContinuation`],
/// after which the program can resolve to a normal [`Grid`].
#[derive(Clone)]
pub struct AdaptiveSource {
    level: StableId,
    program: Arc<dyn AdaptiveProgram>,
}

impl fmt::Debug for AdaptiveSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdaptiveSource")
            .field("level", &self.level)
            .field("program", &self.program)
            .finish()
    }
}

/// A pure program which starts a probe-driven source.
pub trait AdaptiveProgram: fmt::Debug + Send + Sync {
    fn start(&self) -> DiscoverableStep;
}

impl AdaptiveSource {
    #[must_use]
    pub fn new(level: StableId, program: impl AdaptiveProgram + 'static) -> Self {
        Self {
            level,
            program: Arc::new(program),
        }
    }

    #[must_use]
    pub const fn id(&self) -> &StableId {
        &self.level
    }

    #[must_use]
    pub fn start(&self) -> DiscoverableStep {
        self.program.start()
    }
}

impl DiscoverableGrid {
    #[must_use]
    pub fn new(level: StableId, template: String) -> Self {
        Self {
            level,
            template: parse_template(template),
        }
    }

    #[must_use]
    pub const fn id(&self) -> &StableId {
        &self.level
    }

    #[must_use]
    pub fn start(self) -> DiscoverableStep {
        GenericSearch::new(self).next_step()
    }
}

pub enum DiscoverableStep {
    Probe {
        tile: TileSpec,
        continuation: ProbeContinuation,
    },
    Resolved {
        grid: Grid,
        previously_output: Vec<Vec2d>,
    },
    Empty,
    Error(TileSourceError),
}

pub struct ProbeContinuation {
    next: Box<dyn FnOnce(ObservationResult) -> Result<DiscoverableStep, TileSourceError> + Send>,
}

impl ProbeContinuation {
    pub fn new<F>(next: F) -> Self
    where
        F: FnOnce(ObservationResult) -> Result<DiscoverableStep, TileSourceError> + Send + 'static,
    {
        Self {
            next: Box::new(next),
        }
    }

    pub fn submit(self, result: ObservationResult) -> Result<DiscoverableStep, TileSourceError> {
        (self.next)(result)
    }
}

impl fmt::Debug for ProbeContinuation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("probe continuation")
    }
}

impl GenericSearch {
    fn submit(
        mut self,
        point: Vec2d,
        result: ObservationResult,
    ) -> Result<DiscoverableStep, TileSourceError> {
        let can_output = point == Vec2d::default() || self.tile_size.is_some();
        let success = match result {
            ObservationResult::Available { size }
                if size.x > 0 && size.y > 0 && size != Vec2d::square(1) =>
            {
                self.tile_size.get_or_insert(size);
                true
            }
            ObservationResult::Available { size } if size == Vec2d::square(1) => false,
            ObservationResult::Available { .. } => return Err(TileSourceError::InvalidDimensions),
            ObservationResult::Missing => false,
        };
        if success && can_output {
            self.output_points.push(point);
        }
        let first = self.observed.is_empty();
        self.observed.insert(point, success);
        if first {
            self.next_point = Dichotomy2d::first();
        } else {
            let mut next = self.bounds.next(success);
            while let Some(point) = next {
                if let Some(previous) = self.observed.get(&point) {
                    next = self.bounds.next(*previous);
                } else {
                    self.next_point = point;
                    return Ok(self.next_step());
                }
            }
            return self.resolve();
        }
        Ok(self.next_step())
    }
}

#[derive(Debug)]
struct GenericRequests {
    template: Template<Dimension>,
}

impl GridRequests for GenericRequests {
    fn request(&self, tile: GridTile) -> Request {
        Request::new(render_template(
            &self.template,
            tile.coord.column,
            tile.coord.row,
        ))
    }

    fn use_first_tile_as_referer(&self) -> bool {
        false
    }
}

struct GenericSearch {
    source: DiscoverableGrid,
    bounds: Dichotomy2d,
    observed: HashMap<Vec2d, bool>,
    output_points: Vec<Vec2d>,
    next_point: Vec2d,
    next_id: u64,
    tile_size: Option<Vec2d>,
}

impl GenericSearch {
    fn new(source: DiscoverableGrid) -> Self {
        Self {
            source,
            bounds: Dichotomy2d::default(),
            observed: HashMap::new(),
            output_points: Vec::new(),
            next_point: Vec2d::default(),
            next_id: 0,
            tile_size: None,
        }
    }

    fn next_step(self) -> DiscoverableStep {
        let point = self.next_point;
        let size = self.tile_size.unwrap_or_default();
        let Some(destination) = point.checked_mul(size) else {
            return DiscoverableStep::Error(TileSourceError::ArithmeticOverflow);
        };
        let id = self.next_id;
        let template = self.source.template.clone();
        let mut search = self;
        search.next_id = search.next_id.saturating_add(1);
        let role = if point == Vec2d::default() || search.tile_size.is_some() {
            TileRole::ProbeAndOutput
        } else {
            TileRole::Probe
        };
        let tile = TileSpec {
            id: TileId::new(search.source.level.clone(), id),
            request: Request::new(render_template(&template, point.x, point.y)),
            destination,
            expected_size: None,
            processing: super::model::ProcessingRecipe::None,
            role,
        };
        DiscoverableStep::Probe {
            tile,
            continuation: ProbeContinuation::new(move |result| search.submit(point, result)),
        }
    }

    fn resolve(self) -> Result<DiscoverableStep, TileSourceError> {
        let Some(tile_size) = self.tile_size else {
            return Ok(DiscoverableStep::Empty);
        };
        let last = self.bounds.boundary();
        let shape = Vec2d {
            x: last
                .x
                .checked_add(1)
                .ok_or(TileSourceError::ArithmeticOverflow)?,
            y: last
                .y
                .checked_add(1)
                .ok_or(TileSourceError::ArithmeticOverflow)?,
        };
        let image_size = Vec2d {
            x: shape
                .x
                .checked_mul(tile_size.x)
                .ok_or(TileSourceError::ArithmeticOverflow)?,
            y: shape
                .y
                .checked_mul(tile_size.y)
                .ok_or(TileSourceError::ArithmeticOverflow)?,
        };
        let previously_output = self
            .output_points
            .into_iter()
            .map(|point| point.checked_mul(tile_size))
            .collect::<Option<Vec<_>>>()
            .ok_or(TileSourceError::ArithmeticOverflow)?;
        Grid::new(
            self.source.level,
            image_size,
            tile_size,
            Vec2d::default(),
            GenericRequests {
                template: self.source.template,
            },
        )
        .map(|grid| DiscoverableStep::Resolved {
            grid,
            previously_output,
        })
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
            Self::Diagonal(search) => Vec2d::square(search.guess()),
            Self::Orientation(diagonal) => Vec2d::square(*diagonal),
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
                        x: diagonal.saturating_add(1),
                        y: diagonal,
                    })
                },
                |next| Some(Vec2d::square(next)),
            ),
            Self::Orientation(diagonal) => {
                let search = Dichotomy {
                    min: diagonal.saturating_add(u32::from(success)),
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
    use super::*;

    #[test]
    fn generic_template_preserves_syntax_and_padding() {
        let template = parse_template("a/{{ X:03 }}/{{y}}/{{z}}/{{x:bad}}".into());
        assert_eq!(
            render_template(&template, 4, 12),
            "a/004/12/{{z}}/{{x:bad}}"
        );
    }

    #[test]
    fn generic_resolves_to_exact_grid() {
        let existing = ["0,0", "1,0", "2,0", "0,1", "1,1", "2,1"];
        let mut step = DiscoverableGrid::new("level".into(), "{{X}},{{y}}".into()).start();
        for _ in 0..20 {
            step = match step {
                DiscoverableStep::Probe { tile, continuation } => {
                    let result = if existing.contains(&tile.request.uri.as_str()) {
                        ObservationResult::Available {
                            size: Vec2d { x: 4, y: 5 },
                        }
                    } else {
                        ObservationResult::Missing
                    };
                    continuation.submit(result).unwrap()
                }
                DiscoverableStep::Resolved { grid, .. } => {
                    assert_eq!(grid.image_size(), Vec2d { x: 12, y: 10 });
                    assert_eq!(grid.count(), 6);
                    return;
                }
                DiscoverableStep::Empty => panic!("origin exists"),
                DiscoverableStep::Error(error) => panic!("unexpected adaptive error: {error}"),
            };
        }
        panic!("generic search did not resolve");
    }

    #[test]
    fn generic_search_without_tiles_is_empty() {
        let mut step = DiscoverableGrid::new("level".into(), "{{X}},{{Y}}".into()).start();
        for _ in 0..64 {
            step = match step {
                DiscoverableStep::Probe { continuation, .. } => {
                    continuation.submit(ObservationResult::Missing).unwrap()
                }
                DiscoverableStep::Empty => return,
                DiscoverableStep::Resolved { .. } => panic!("missing tiles must not resolve"),
                DiscoverableStep::Error(error) => panic!("unexpected adaptive error: {error}"),
            };
        }
        panic!("generic search did not become empty");
    }

    #[test]
    fn coordinate_overflow_becomes_an_adaptive_error() {
        let DiscoverableStep::Probe { continuation, .. } =
            DiscoverableGrid::new("level".into(), "{{X}},{{Y}}".into()).start()
        else {
            panic!("generic search must begin with a probe")
        };
        let DiscoverableStep::Probe { continuation, .. } = continuation
            .submit(ObservationResult::Available {
                size: Vec2d::square(u32::MAX),
            })
            .unwrap()
        else {
            panic!("generic search must continue probing")
        };
        assert!(matches!(
            continuation
                .submit(ObservationResult::Available {
                    size: Vec2d::square(u32::MAX),
                })
                .unwrap(),
            DiscoverableStep::Error(TileSourceError::ArithmeticOverflow)
        ));
    }
}
