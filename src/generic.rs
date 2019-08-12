use crate::dezoomer::{Dezoomer, DezoomerError, DezoomerInput, ZoomLevels};

pub fn all_dezoomers(include_generic: bool) -> Vec<Box<dyn Dezoomer>> {
    let mut dezoomers: Vec<Box<dyn Dezoomer>> = vec![
        Box::new(crate::custom_yaml::CustomDezoomer::default()),
        Box::new(crate::google_arts_and_culture::GAPDezoomer::default()),
        Box::new(crate::zoomify::ZoomifyDezoomer::default()),
    ];
    if include_generic {
        dezoomers.push(Box::new(GenericDezoomer::default()))
    }
    dezoomers
}

pub struct GenericDezoomer {
    dezoomers: Vec<Box<dyn Dezoomer>>,
}

impl Default for GenericDezoomer {
    fn default() -> Self {
        GenericDezoomer {
            dezoomers: all_dezoomers(false),
        }
    }
}

impl Dezoomer for GenericDezoomer {
    fn name(&self) -> &'static str {
        "generic"
    }

    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        let mut errs = vec![];
        let mut successes = Vec::new();
        let mut needs_uri = None;
        // TO DO: Use drain_filter when it is stabilized
        let mut i = 0;
        while i != self.dezoomers.len() {
            let keep = match self.dezoomers[i].zoom_levels(data) {
                Ok(mut levels) => {
                    successes.append(&mut levels);
                    true
                }
                Err(e @ DezoomerError::NeedsData { .. }) => {
                    needs_uri = Some(e);
                    true
                }
                Err(e) => {
                    errs.push(e);
                    false
                }
            };
            if keep {
                i += 1
            } else {
                self.dezoomers.remove(i);
            }
        }
        if successes.is_empty() {
            Err(needs_uri.unwrap_or_else(|| DezoomerError::wrap(GenericError(errs))))
        } else {
            Ok(successes)
        }
    }
}

#[derive(Debug)]
pub struct GenericError(Vec<DezoomerError>);

impl std::error::Error for GenericError {}

impl std::fmt::Display for GenericError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if self.0.is_empty() {
            return writeln!(f, "No dezoomer!");
        }
        writeln!(
            f,
            "Tried all of the dezoomers, none succeeded. They returned the following errors:\n"
        )?;
        for e in self.0.iter() {
            writeln!(f, " - {}", e)?;
        }
        Ok(())
    }
}
