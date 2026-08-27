//! Resolution of generic origin-anchored solid tile rectangles.

use std::collections::HashMap;
use std::sync::LazyLock;

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
    Regex::new(r"(?xi)\{\{\s*(?P<dimension>x|y)(?::0(?P<zeroes>\d+))?\s*\}\}")
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
        let dimension = if captures["dimension"].eq_ignore_ascii_case("x") {
            Dimension::X
        } else {
            Dimension::Y
        };
        let padding = captures
            .name("zeroes")
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
}

pub struct ProbeContinuation {
    search: GenericSearch,
    point: Vec2d,
}

impl ProbeContinuation {
    pub fn submit(
        mut self,
        result: ObservationResult,
    ) -> Result<DiscoverableStep, TileSourceError> {
        let success = match result {
            ObservationResult::Available { size } if size.x > 0 && size.y > 0 => {
                self.search.tile_size.get_or_insert(size);
                true
            }
            ObservationResult::Available { .. } => return Err(TileSourceError::InvalidDimensions),
            ObservationResult::Missing => false,
        };
        let first = self.search.observed.is_empty();
        self.search.observed.insert(self.point, success);
        if first {
            if success {
                self.search.next_point = Dichotomy2d::first();
            } else {
                return Ok(DiscoverableStep::Empty);
            }
        } else {
            let mut next = self.search.bounds.next(success);
            while let Some(point) = next {
                if let Some(previous) = self.search.observed.get(&point) {
                    next = self.search.bounds.next(*previous);
                } else {
                    self.search.next_point = point;
                    return Ok(self.search.next_step());
                }
            }
            return self.search.resolve();
        }
        Ok(self.search.next_step())
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
            next_point: Vec2d::default(),
            next_id: 0,
            tile_size: None,
        }
    }

    fn next_step(mut self) -> DiscoverableStep {
        let point = self.next_point;
        let size = self.tile_size.unwrap_or_default();
        let tile = TileSpec {
            id: TileId::new(self.source.level.clone(), self.next_id),
            request: Request::new(render_template(&self.source.template, point.x, point.y)),
            destination: point * size,
            expected_size: None,
            processing: super::model::ProcessingRecipe::None,
            role: TileRole::ProbeAndOutput,
        };
        self.next_id += 1;
        DiscoverableStep::Probe {
            tile,
            continuation: ProbeContinuation {
                search: self,
                point,
            },
        }
    }

    fn resolve(self) -> Result<DiscoverableStep, TileSourceError> {
        let tile_size = self.tile_size.expect("a resolved search found the origin");
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
            .observed
            .iter()
            .filter_map(|(point, available)| available.then_some(*point * tile_size))
            .collect();
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
                        x: diagonal + 1,
                        y: diagonal,
                    })
                },
                |next| Some(Vec2d::square(next)),
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
            };
        }
        panic!("generic search did not resolve");
    }
}
