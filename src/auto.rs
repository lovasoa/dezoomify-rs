use log::debug;

use crate::dezoomer::{Dezoomer, DezoomerError, DezoomerInput, Images, ZoomLevel, ZoomLevels};
use crate::errors::DezoomerError::NeedsData;

/// Reorder dezoomers to prioritize those most likely to handle the given URL
pub fn prioritize_dezoomers_for_url(
    url: &str,
    mut dezoomers: Vec<Box<dyn Dezoomer>>,
) -> Vec<Box<dyn Dezoomer>> {
    // Define URL patterns and their preferred dezoomers
    let patterns = [
        ("info.json", "iiif"),
        ("iiif", "iiif"),
        ("manifest.json", "iiif"),
        (".dzi", "deepzoom"),
        ("_files/", "deepzoom"),
        ("?FIF", "IIPImage"),
        ("tiles.xml", "krpano"),
        ("ImageProperties.xml", "zoomify"),
        ("TileGroup", "zoomify"),
        ("digitalcollections.nypl.org", "nypl"),
        ("{{", "generic"),
    ];

    // Find the best matching dezoomer
    let preferred_dezoomer = patterns
        .iter()
        .find(|(pattern, _)| url.contains(pattern))
        .map(|(_, dezoomer)| *dezoomer);

    if let Some(preferred_name) = preferred_dezoomer {
        debug!(
            "URL '{}' appears to match '{}' dezoomer, prioritizing it",
            url, preferred_name
        );

        // Move the preferred dezoomer to the front
        let preferred_idx = dezoomers.iter().position(|d| d.name() == preferred_name);
        if let Some(idx) = preferred_idx {
            let preferred = dezoomers.remove(idx);
            dezoomers.insert(0, preferred);
        }
    }

    dezoomers
}

pub fn all_dezoomers(include_generic: bool) -> Vec<Box<dyn Dezoomer>> {
    let mut dezoomers: Vec<Box<dyn Dezoomer>> = vec![
        Box::<crate::custom_yaml::CustomDezoomer>::default(),
        Box::<crate::google_arts_and_culture::GAPDezoomer>::default(),
        Box::<crate::zoomify::ZoomifyDezoomer>::default(),
        Box::<crate::iiif::IIIF>::default(),
        Box::<crate::dzi::DziDezoomer>::default(),
        Box::<crate::generic::GenericDezoomer>::default(),
        Box::<crate::pff::PFF>::default(),
        Box::<crate::krpano::KrpanoDezoomer>::default(),
        Box::<crate::iipimage::IIPImage>::default(),
        Box::<crate::nypl::NYPLImage>::default(),
        Box::<crate::bulk_text::BulkTextDezoomer>::default(),
    ];
    if include_generic {
        dezoomers.push(Box::<AutoDezoomer>::default())
    }
    dezoomers
}

pub struct AutoDezoomer {
    dezoomers: Vec<Box<dyn Dezoomer>>,
    errors: Vec<(&'static str, DezoomerError)>,
    successes: Vec<ZoomLevel>,
    needs_uris: Vec<String>,
    prioritized_for_url: Option<String>,
}

impl Default for AutoDezoomer {
    fn default() -> Self {
        AutoDezoomer {
            dezoomers: all_dezoomers(false),
            errors: vec![],
            successes: vec![],
            needs_uris: vec![],
            prioritized_for_url: None,
        }
    }
}

impl AutoDezoomer {
    /// Prioritize dezoomers for a specific URL if not already done
    fn prioritize_for_url_if_needed(&mut self, url: &str) {
        if self.prioritized_for_url.as_ref() != Some(&url.to_string()) {
            debug!("Prioritizing dezoomers for URL: {}", url);
            let dezoomers = std::mem::take(&mut self.dezoomers);
            self.dezoomers = prioritize_dezoomers_for_url(url, dezoomers);
            self.prioritized_for_url = Some(url.to_string());
        }
    }
}

impl Dezoomer for AutoDezoomer {
    fn name(&self) -> &'static str {
        "auto"
    }

    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        // Prioritize dezoomers based on the URL pattern
        self.prioritize_for_url_if_needed(&data.uri);

        // TO DO: Use drain_filter when it is stabilized
        let mut i = 0;
        while i != self.dezoomers.len() {
            let dezoomer = &mut self.dezoomers[i];
            let keep = match dezoomer.zoom_levels(data) {
                Ok(mut levels) => {
                    debug!(
                        "dezoomer '{}' found {} zoom levels",
                        dezoomer.name(),
                        levels.len()
                    );
                    self.successes.append(&mut levels);
                    false
                }
                Err(DezoomerError::NeedsData { uri }) => {
                    debug!("dezoomer '{}' requested to load {}", dezoomer.name(), uri);
                    if !self.needs_uris.contains(&uri) {
                        self.needs_uris.push(uri);
                    }
                    true
                }
                Err(e) => {
                    debug!("{} cannot process this image: {}", dezoomer.name(), e);
                    self.errors.push((dezoomer.name(), e));
                    false
                }
            };
            if keep {
                i += 1
            } else {
                self.dezoomers.remove(i);
            }
        }
        if let Some(uri) = self.needs_uris.pop() {
            Err(NeedsData { uri })
        } else if self.successes.is_empty() {
            debug!("No dezoomer can dezoom {:?}", data.uri);
            let errs = std::mem::take(&mut self.errors);
            Err(DezoomerError::wrap(AutoDezoomerError(errs)))
        } else {
            let successes = std::mem::take(&mut self.successes);
            Ok(successes)
        }
    }

    fn images(&mut self, data: &DezoomerInput) -> Result<Images, DezoomerError> {
        // Prioritize dezoomers based on the URL pattern
        self.prioritize_for_url_if_needed(&data.uri);

        // TO DO: Use drain_filter when it is stabilized
        let mut i = 0;
        while i != self.dezoomers.len() {
            let dezoomer = &mut self.dezoomers[i];
            let keep = match dezoomer.images(data) {
                Ok(result) => {
                    debug!(
                        "dezoomer '{}' successfully processed the input",
                        dezoomer.name()
                    );
                    return Ok(result);
                }
                Err(DezoomerError::NeedsData { uri }) => {
                    debug!("dezoomer '{}' requested to load {}", dezoomer.name(), uri);
                    if !self.needs_uris.contains(&uri) {
                        self.needs_uris.push(uri);
                    }
                    true
                }
                Err(e) => {
                    debug!("{} cannot process this image: {}", dezoomer.name(), e);
                    self.errors.push((dezoomer.name(), e));
                    false
                }
            };
            if keep {
                i += 1
            } else {
                self.dezoomers.remove(i);
            }
        }
        if let Some(uri) = self.needs_uris.pop() {
            Err(NeedsData { uri })
        } else {
            debug!("No dezoomer can process {:?}", data.uri);
            let errs = std::mem::take(&mut self.errors);
            Err(DezoomerError::wrap(AutoDezoomerError(errs)))
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
            writeln!(f, " - {dezoomer_name}: {err}")?;
        }
        writeln!(
            f,
            "\n\
        dezoomify-rs expects a zoomable image meta-information file URL. \
        To find this URL, you can use the dezoomify browser extension, which you can download at\n\
         - https://lovasoa.github.io/dezoomify-extension/ \n\
        If this doesn't help, then your image may be in a format that is not yet supported by dezoomify-rs.\n\
        You can ask for a new format to be supported by opening a new issue on \
        https://github.com/lovasoa/dezoomify-rs/issues"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prioritize_dezoomers_for_url() {
        // Test IIIF URL prioritization
        let iiif_url = "https://example.com/iiif/service/info.json";
        let dezoomers = all_dezoomers(false);
        let prioritized = prioritize_dezoomers_for_url(iiif_url, dezoomers);

        // IIIF dezoomer should be first
        assert_eq!(prioritized[0].name(), "iiif");

        // Test Zoomify URL prioritization
        let zoomify_url = "https://example.com/ImageProperties.xml";
        let dezoomers = all_dezoomers(false);
        let prioritized = prioritize_dezoomers_for_url(zoomify_url, dezoomers);

        // Zoomify dezoomer should be first
        assert_eq!(prioritized[0].name(), "zoomify");

        // Test DeepZoom URL prioritization
        let dzi_url = "https://example.com/image.dzi";
        let dezoomers = all_dezoomers(false);
        let prioritized = prioritize_dezoomers_for_url(dzi_url, dezoomers);

        // DeepZoom dezoomer should be first
        assert_eq!(prioritized[0].name(), "deepzoom");

        // Test unknown URL - should preserve original order
        let unknown_url = "https://example.com/unknown.xyz";
        let dezoomers = all_dezoomers(false);
        let original_first = dezoomers[0].name();
        let prioritized = prioritize_dezoomers_for_url(unknown_url, dezoomers);

        // Should preserve original order for unknown URLs
        assert_eq!(prioritized[0].name(), original_first);
    }

    #[test]
    fn test_prioritize_dezoomers_edge_cases() {
        // Test empty URL
        let empty_url = "";
        let dezoomers = all_dezoomers(false);
        let original_first = dezoomers[0].name();
        let prioritized = prioritize_dezoomers_for_url(empty_url, dezoomers);
        assert_eq!(prioritized[0].name(), original_first);

        // Test multiple pattern matches - should prioritize first match
        let iiif_info_url = "https://example.com/iiif/service/info.json";
        let dezoomers = all_dezoomers(false);
        let prioritized = prioritize_dezoomers_for_url(iiif_info_url, dezoomers);
        assert_eq!(prioritized[0].name(), "iiif");

        // Test case insensitive matching
        let zoomify_upper = "https://example.com/IMAGEPROPERTIES.XML";
        let dezoomers = all_dezoomers(false);
        let original_first = dezoomers[0].name();
        let prioritized = prioritize_dezoomers_for_url(zoomify_upper, dezoomers);
        // Current implementation is case-sensitive, so uppercase won't match
        assert_eq!(prioritized[0].name(), original_first);
    }
}
