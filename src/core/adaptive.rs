//! Pure, observation-driven tile planning for formats whose bounds are unknown.

use std::collections::{HashSet, VecDeque};
use std::error::Error;
use std::fmt::{self, Write};

use super::model::{
    Dimensions, Point, ProcessingRecipe, Region, RequestSpec, StableId, TileId, TileRole, TileSpec,
};

/// An observation supplied by an application after inspecting one probe tile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TileObservation {
    pub id: TileId,
    pub result: ObservationResult,
}

impl TileObservation {
    #[must_use]
    pub fn success(id: TileId, dimensions: Dimensions) -> Self {
        Self {
            id,
            result: ObservationResult::Success { dimensions },
        }
    }

    #[must_use]
    pub fn failure(id: TileId) -> Self {
        Self {
            id,
            result: ObservationResult::Failure,
        }
    }
}

/// Intrinsic information learned from a probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationResult {
    Success { dimensions: Dimensions },
    Failure,
}

/// Errors caused by invalid adaptive-program sequencing or observations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdaptiveError {
    ZeroCapacity,
    PendingObservations,
    NoPendingObservations,
    EmptyObservationBatch,
    UnknownObservation(TileId),
    DuplicateObservation(TileId),
    WrongObservationRole(TileId),
    InvalidDimensions(TileId),
    ArithmeticOverflow,
}

impl fmt::Display for AdaptiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => f.write_str("adaptive batch capacity must be greater than zero"),
            Self::PendingObservations => {
                f.write_str("cannot request more probes while observations are pending")
            }
            Self::NoPendingObservations => f.write_str("no probe observations are pending"),
            Self::EmptyObservationBatch => {
                f.write_str("at least one probe observation is required")
            }
            Self::UnknownObservation(id) => write!(f, "observation is not pending: {id:?}"),
            Self::DuplicateObservation(id) => write!(f, "duplicate observation: {id:?}"),
            Self::WrongObservationRole(id) => write!(f, "observation is not a probe: {id:?}"),
            Self::InvalidDimensions(id) => write!(f, "probe returned invalid dimensions: {id:?}"),
            Self::ArithmeticOverflow => f.write_str("adaptive tile geometry overflowed u32"),
        }
    }
}

impl Error for AdaptiveError {}

/// A pure Generic-style adaptive planner.
///
/// The planner emits one probe at a time because each dichotomy transition
/// depends on that probe's success.  Once the bounds are established, the
/// remaining output tiles are emitted in canonical row-major batches.
#[derive(Clone, Debug)]
pub struct AdaptiveProgram {
    level: StableId,
    url_template: String,
    dichotomy: Dichotomy2d,
    last_tile: Point,
    tile_size: Option<Dimensions>,
    image_size: Option<Dimensions>,
    done: HashSet<Point>,
    pending_output: VecDeque<TileSpec>,
    pending_probe: Option<ProbeBatch>,
    next_probe_ordinal: u64,
    finished: bool,
}

#[derive(Clone, Debug)]
struct ProbeBatch {
    id: TileId,
    point: Point,
}

impl AdaptiveProgram {
    #[must_use]
    pub fn new(level: impl Into<StableId>, url_template: impl Into<String>) -> Self {
        Self {
            level: level.into(),
            url_template: url_template.into(),
            dichotomy: Dichotomy2d::default(),
            last_tile: Point::default(),
            tile_size: None,
            image_size: None,
            done: HashSet::new(),
            pending_output: VecDeque::new(),
            pending_probe: None,
            next_probe_ordinal: 0,
            finished: false,
        }
    }

    /// Returns ready probes or, after bounds are known, output tile specs.
    ///
    /// # Errors
    ///
    /// Returns [`AdaptiveError::ZeroCapacity`] for zero capacity or
    /// [`AdaptiveError::PendingObservations`] when the previous probe has not
    /// been submitted yet.
    pub fn take_ready(&mut self, capacity: usize) -> Result<Option<Vec<TileSpec>>, AdaptiveError> {
        if capacity == 0 {
            return Err(AdaptiveError::ZeroCapacity);
        }
        if self.pending_probe.is_some() {
            return Err(AdaptiveError::PendingObservations);
        }
        if !self.pending_output.is_empty() {
            let count = capacity.min(self.pending_output.len());
            let result = self.pending_output.drain(..count).collect();
            return Ok(Some(result));
        }
        if self.finished {
            return Ok(None);
        }

        let point = if self.done.is_empty() {
            Point::default()
        } else {
            // The dichotomy always leaves a next point in `advance`; this is
            // only reached for the initial probe.
            self.last_tile
        };
        let id = TileId::new(self.level.clone(), TileRole::Probe, self.next_probe_ordinal);
        self.next_probe_ordinal = self.next_probe_ordinal.saturating_add(1);
        self.pending_probe = Some(ProbeBatch {
            id: id.clone(),
            point,
        });
        Ok(Some(vec![self.probe_spec(id, point)]))
    }

    /// Advances the program using observations for the currently pending probe.
    ///
    /// # Errors
    ///
    /// Returns a typed error when observations are missing, duplicated, do not
    /// identify the pending probe, or report invalid dimensions.
    pub fn submit(
        &mut self,
        observations: impl IntoIterator<Item = TileObservation>,
    ) -> Result<(), AdaptiveError> {
        let Some(batch) = self.pending_probe.take() else {
            return Err(AdaptiveError::NoPendingObservations);
        };
        let observations: Vec<_> = observations.into_iter().collect();
        if observations.is_empty() {
            self.pending_probe = Some(batch);
            return Err(AdaptiveError::EmptyObservationBatch);
        }
        if observations.len() != 1 {
            self.pending_probe = Some(batch);
            if observations[1].id == observations[0].id {
                return Err(AdaptiveError::DuplicateObservation(
                    observations[1].id.clone(),
                ));
            }
            return Err(AdaptiveError::UnknownObservation(
                observations[1].id.clone(),
            ));
        }
        let observation = &observations[0];
        if observation.id != batch.id {
            let expected_ordinal = batch.id.ordinal;
            self.pending_probe = Some(batch);
            if observation.id.level == self.level && observation.id.ordinal == expected_ordinal {
                return Err(AdaptiveError::WrongObservationRole(observation.id.clone()));
            }
            return Err(AdaptiveError::UnknownObservation(observation.id.clone()));
        }
        let (success, dimensions) = match observation.result {
            ObservationResult::Success { dimensions } if !dimensions.is_empty() => {
                (true, Some(dimensions))
            }
            ObservationResult::Success { .. } => {
                self.pending_probe = Some(batch);
                return Err(AdaptiveError::InvalidDimensions(observation.id.clone()));
            }
            ObservationResult::Failure => (false, None),
        };

        if let Some(dimensions) = dimensions {
            self.tile_size.get_or_insert(dimensions);
        }
        self.last_tile = batch.point;
        self.done.insert(batch.point);
        if let Some(next) = self.dichotomy.next(success) {
            self.last_tile = next;
            return Ok(());
        }
        self.finish_bounds()
    }

    #[must_use]
    pub fn image_size(&self) -> Option<Dimensions> {
        self.image_size
    }

    fn probe_spec(&self, id: TileId, point: Point) -> TileSpec {
        let tile_size = self.tile_size.unwrap_or_default();
        let origin = Point::new(
            point.x.saturating_mul(tile_size.width),
            point.y.saturating_mul(tile_size.height),
        );
        let region = Region::new(origin, tile_size);
        TileSpec {
            id,
            request: RequestSpec::new(render_template(&self.url_template, point.x, point.y)),
            source_region: region,
            destination_region: region,
            expected_size: None,
            processing: ProcessingRecipe::None,
            role: TileRole::Probe,
        }
    }

    fn finish_bounds(&mut self) -> Result<(), AdaptiveError> {
        let Some(tile_size) = self.tile_size else {
            self.finished = true;
            return Ok(());
        };
        let width = u32::try_from(
            (u64::from(self.last_tile.x) + 1)
                .checked_mul(u64::from(tile_size.width))
                .ok_or(AdaptiveError::ArithmeticOverflow)?,
        )
        .map_err(|_| AdaptiveError::ArithmeticOverflow)?;
        let height = u32::try_from(
            (u64::from(self.last_tile.y) + 1)
                .checked_mul(u64::from(tile_size.height))
                .ok_or(AdaptiveError::ArithmeticOverflow)?,
        )
        .map_err(|_| AdaptiveError::ArithmeticOverflow)?;
        self.image_size = Some(Dimensions::new(width, height));

        let mut ordinal = 0_u64;
        for y in 0..=self.last_tile.y {
            for x in 0..=self.last_tile.x {
                let point = Point::new(x, y);
                if self.done.contains(&point) {
                    continue;
                }
                let origin = Point::new(
                    x.checked_mul(tile_size.width)
                        .ok_or(AdaptiveError::ArithmeticOverflow)?,
                    y.checked_mul(tile_size.height)
                        .ok_or(AdaptiveError::ArithmeticOverflow)?,
                );
                let region = Region::new(origin, tile_size);
                self.pending_output.push_back(TileSpec {
                    id: TileId::new(self.level.clone(), TileRole::Output, ordinal),
                    request: RequestSpec::new(render_template(&self.url_template, x, y)),
                    source_region: region,
                    destination_region: region,
                    expected_size: Some(tile_size),
                    processing: ProcessingRecipe::None,
                    role: TileRole::Output,
                });
                ordinal = ordinal.saturating_add(1);
            }
        }
        self.done.clear();
        // `finished` means no further probes will ever be generated.  Output
        // specs may still remain in `pending_output` and are drained first by
        // `take_ready`.
        self.finished = true;
        Ok(())
    }
}

fn render_template(template: &str, x: u32, y: u32) -> String {
    let mut result = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        result.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        let Some(end) = after_open.find("}}") else {
            result.push_str(&rest[start..]);
            return result;
        };
        let expression = after_open[..end].trim();
        let (dimension, width) =
            expression
                .split_once(':')
                .map_or((expression, 0), |(dimension, padding)| {
                    (
                        dimension.trim(),
                        padding
                            .strip_prefix('0')
                            .and_then(|n| n.parse().ok())
                            .unwrap_or(0),
                    )
                });
        let value = match dimension.to_ascii_lowercase().as_str() {
            "x" => Some(x),
            "y" => Some(y),
            _ => None,
        };
        if let Some(value) = value {
            if width == 0 {
                result.push_str(&value.to_string());
            } else {
                let _ = write!(result, "{value:0width$}");
            }
        } else {
            result.push_str(&rest[start..start + 2 + end + 2]);
        }
        rest = &after_open[end + 2..];
    }
    result.push_str(rest);
    result
}

#[derive(Clone, Debug, Default)]
struct Dichotomy {
    min: u32,
    max: Option<u32>,
}

impl Dichotomy {
    fn best_guess(&self) -> u32 {
        self.max
            .map_or(self.min * 3 + 1, |max| u32::midpoint(max, self.min))
    }

    fn next(&mut self, previous_success: bool) -> Option<u32> {
        let last_guess = self.best_guess();
        if previous_success {
            self.min = last_guess;
        } else {
            self.max = Some(last_guess);
        }
        let next_guess = self.best_guess();
        (next_guess != last_guess).then_some(next_guess)
    }
}

#[derive(Clone, Debug)]
enum Dichotomy2d {
    Diagonal(Dichotomy),
    Orientation {
        diagonal: u32,
    },
    LastDim {
        diagonal: u32,
        is_landscape: bool,
        last_dim: Dichotomy,
    },
}

impl Default for Dichotomy2d {
    fn default() -> Self {
        Self::Diagonal(Dichotomy::default())
    }
}

impl Dichotomy2d {
    fn next(&mut self, previous_success: bool) -> Option<Point> {
        let mut transition = None;
        let result = match self {
            Self::Diagonal(dichotomy) => {
                if let Some(next) = dichotomy.next(previous_success) {
                    Some(Point::new(next, next))
                } else {
                    let diagonal = dichotomy.best_guess();
                    transition = Some(Self::Orientation { diagonal });
                    Some(Point::new(diagonal + 1, diagonal))
                }
            }
            Self::Orientation { diagonal } => {
                let dichotomy = Dichotomy {
                    min: *diagonal + u32::from(previous_success),
                    max: None,
                };
                let best = dichotomy.best_guess();
                transition = Some(Self::LastDim {
                    diagonal: *diagonal,
                    is_landscape: previous_success,
                    last_dim: dichotomy,
                });
                if previous_success {
                    Some(Point::new(best, *diagonal))
                } else {
                    Some(Point::new(*diagonal, best))
                }
            }
            Self::LastDim {
                diagonal,
                is_landscape,
                last_dim,
            } => last_dim.next(previous_success).map(|next| {
                if *is_landscape {
                    Point::new(next, *diagonal)
                } else {
                    Point::new(*diagonal, next)
                }
            }),
        };
        if let Some(transition) = transition {
            *self = transition;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn success_for(spec: &TileSpec, width: u32, height: u32) -> TileObservation {
        TileObservation::success(spec.id.clone(), Dimensions::new(width, height))
    }

    #[test]
    fn advances_only_from_submitted_observations() {
        let mut program = AdaptiveProgram::new("level", "memory://{{X}},{{Y}}");
        let mut output = Vec::new();
        let mut steps = 0;
        while let Some(batch) = program.take_ready(8).unwrap() {
            steps += 1;
            assert!(steps < 20);
            if batch[0].role == TileRole::Probe {
                let success = [
                    "memory://0,0",
                    "memory://1,0",
                    "memory://2,0",
                    "memory://0,1",
                    "memory://1,1",
                    "memory://2,1",
                ]
                .contains(&batch[0].request.uri.as_str());
                let observation = if success {
                    success_for(&batch[0], 4, 5)
                } else {
                    TileObservation::failure(batch[0].id.clone())
                };
                program.submit([observation]).unwrap();
            } else {
                output.extend(batch);
            }
        }
        let urls: HashSet<_> = output
            .iter()
            .map(|spec| spec.request.uri.as_str())
            .collect();
        assert!(urls.iter().all(|url| {
            [
                "memory://0,0",
                "memory://1,0",
                "memory://2,0",
                "memory://0,1",
                "memory://1,1",
                "memory://2,1",
            ]
            .contains(url)
        }));
        assert!(!urls.contains("memory://3,0"));
        assert!(!urls.contains("memory://0,2"));
        assert!(output.iter().all(|spec| spec.role == TileRole::Output));
    }

    #[test]
    fn sequencing_and_observation_identity_are_typed() {
        let mut program = AdaptiveProgram::new("level", "memory://{{x}},{{y}}");
        let batch = program.take_ready(1).unwrap().unwrap();
        assert_eq!(
            program.take_ready(1),
            Err(AdaptiveError::PendingObservations)
        );
        assert_eq!(
            program.submit([]),
            Err(AdaptiveError::EmptyObservationBatch)
        );
        let wrong = TileId::new(
            StableId::from("level"),
            TileRole::Output,
            batch[0].id.ordinal,
        );
        assert_eq!(
            program.submit([TileObservation::failure(wrong.clone())]),
            Err(AdaptiveError::WrongObservationRole(wrong))
        );
        let unknown = TileId::new(StableId::from("other"), TileRole::Probe, 0);
        assert_eq!(
            program.submit([TileObservation::failure(unknown.clone())]),
            Err(AdaptiveError::UnknownObservation(unknown))
        );
        let duplicate = TileObservation::failure(batch[0].id.clone());
        assert_eq!(
            program.submit([duplicate.clone(), duplicate]),
            Err(AdaptiveError::DuplicateObservation(batch[0].id.clone()))
        );
        program
            .submit([TileObservation::failure(batch[0].id.clone())])
            .unwrap();
    }

    #[test]
    fn independent_programs_do_not_share_probe_state() {
        let mut first = AdaptiveProgram::new("level-a", "memory://a/{{x}},{{y}}");
        let mut second = AdaptiveProgram::new("level-b", "memory://b/{{x}},{{y}}");
        let first_probe = first.take_ready(1).unwrap().unwrap().remove(0);
        let second_probe = second.take_ready(1).unwrap().unwrap().remove(0);
        assert_ne!(first_probe.id, second_probe.id);
        first
            .submit([TileObservation::failure(first_probe.id)])
            .unwrap();
        let next_second = second.take_ready(1);
        assert_eq!(next_second, Err(AdaptiveError::PendingObservations));
    }

    #[test]
    fn template_padding_and_case_are_preserved() {
        let mut program = AdaptiveProgram::new("level", "memory://{{X:05}}/{{y}}");
        let probe = program.take_ready(1).unwrap().unwrap();
        assert_eq!(probe[0].request.uri, "memory://00000/0");
    }
}
