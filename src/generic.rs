use crate::dezoomer::{Dezoomer, DezoomerError, DezoomerInput, ZoomLevels};

pub const ALL_DEZOOMERS: &'static [Dezoomer] = &[
    crate::custom_yaml::DEZOOMER
];

fn dezoom_fn(data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
    let mut errs = Vec::new();
    let mut successes = Vec::new();
    for dezoomer in ALL_DEZOOMERS {
        match dezoomer.tile_refs(data) {
            Ok(mut levels) => {
                successes.append(&mut levels);
            }
            Err(e) => {
                errs.push(e)
            }
        }
    }
    if successes.is_empty() {
        let need_data = errs.into_iter()
            .find_map(|e| {
                match e {
                    DezoomerError::NeedsData { .. } => Some(e),
                    _ => None
                }
            });
        Err(need_data.unwrap_or(DezoomerError::WrongDezoomer))
    } else {
        Ok(successes)
    }
}

pub const DEZOOMER: Dezoomer = Dezoomer {
    name: "generic",
    dezoom_fn,
};