use crate::dezoomer::{Dezoomer, DezoomerError, DezoomerInput, ZoomLevels};
use log::{info, debug};

pub fn all_dezoomers(include_generic: bool) -> Vec<Box<dyn Dezoomer>> {
    let mut dezoomers: Vec<Box<dyn Dezoomer>> = vec![
        Box::new(crate::custom_yaml::CustomDezoomer::default()),
        Box::new(crate::google_arts_and_culture::GAPDezoomer::default()),
        Box::new(crate::zoomify::ZoomifyDezoomer::default()),
        Box::new(crate::iiif::IIIF::default()),
        Box::new(crate::dzi::DziDezoomer::default()),
        Box::new(crate::generic::GenericDezoomer::default()),
        Box::new(crate::pff::PFF::default()),
        Box::new(crate::krpano::KrpanoDezoomer::default()),
    ];
    if include_generic {
        dezoomers.push(Box::new(AutoDezoomer::default()))
    }
    dezoomers
}

pub struct AutoDezoomer {
    dezoomers: Vec<Box<dyn Dezoomer>>,
}

impl Default for AutoDezoomer {
    fn default() -> Self {
        AutoDezoomer {
            dezoomers: all_dezoomers(false),
        }
    }
}

impl Dezoomer for AutoDezoomer {
    fn name(&self) -> &'static str {
        "auto"
    }

    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        let mut errs = vec![];
        let mut successes = Vec::new();
        let mut needs_uri = None;
        // TO DO: Use drain_filter when it is stabilized
        let mut i = 0;
        while i != self.dezoomers.len() {
            let dezoomer = &mut self.dezoomers[i];
            let keep = match dezoomer.zoom_levels(data) {
                Ok(mut levels) => {
                    successes.append(&mut levels);
                    true
                }
                Err(e @ DezoomerError::NeedsData { .. }) => {
                    debug!("{} requested more data: {}", dezoomer.name(), e);
                    needs_uri = Some(e);
                    true
                }
                Err(e) => {
                    debug!("{} cannot process this image: {}", dezoomer.name(), e);
                    errs.push((dezoomer.name(), e));
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
            info!("No dezoomer can dezoom {:?}", data.uri);
            Err(needs_uri.unwrap_or_else(|| DezoomerError::wrap(AutoDezoomerError(errs))))
        } else {
            Ok(successes)
        }
    }
}

#[derive(Debug)]
pub struct AutoDezoomerError(Vec<(&'static str, DezoomerError)>);

impl std::error::Error for AutoDezoomerError {}

impl std::fmt::Display for AutoDezoomerError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if self.0.is_empty() {
            return writeln!(f, "No dezoomer!");
        }
        writeln!(
            f,
            "Tried all of the dezoomers, none succeeded. They returned the following errors:\n"
        )?;
        for (dezoomer_name, err) in self.0.iter() {
            writeln!(f, " - {}: {}", dezoomer_name, err)?;
        }
        Ok(())
    }
}
