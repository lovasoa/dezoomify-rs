use custom_error::custom_error;

use crate::dezoomer::{Dezoomer, DezoomerError, DezoomerInput, ZoomLevels};

pub const ALL_DEZOOMERS: &'static [Dezoomer] = &[
    crate::custom_yaml::DEZOOMER
];

custom_error! {
pub GenericDezoomerError {uri:String} = "Tried all dezoomers, but none can open '{uri}'"
}

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
        if let Some(need_data) = need_data {
            Err(need_data)
        } else {
            let uri = data.uri.clone();
            Err(DezoomerError::wrap(GenericDezoomerError { uri }))
        }
    } else {
        Ok(successes)
    }
}

pub const DEZOOMER: Dezoomer = Dezoomer {
    name: "generic",
    dezoom_fn,
};