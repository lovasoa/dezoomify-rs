//! Automatic format detection and metadata resolution.
//!
//! Each candidate dezoomer is advanced independently and records the exact URI
//! it is waiting for. Requests for the same URI are queued only once, so one
//! fetched response is delivered to every candidate waiting for it. Candidates
//! waiting for another URI remain paused and never see unrelated metadata.
//!
//! `MetadataResolver` drives the `NeedsData` state machine and caches fetched
//! metadata by its exact URI. Repeated requests, including requests made by a
//! later deferred-image resolution, therefore reuse the first response instead
//! of performing another download.

use std::collections::HashMap;

use log::debug;

use crate::dezoomer::{Dezoomer, DezoomerError, DezoomerInput, Images, PageContents};
use crate::errors::DezoomerError::NeedsData;
use crate::network::fetch_uri;

pub(crate) struct MetadataResolver<'a> {
    http: &'a reqwest::Client,
    cache: HashMap<String, Vec<u8>>,
}

impl<'a> MetadataResolver<'a> {
    pub(crate) fn new(http: &'a reqwest::Client) -> Self {
        Self {
            http,
            cache: HashMap::new(),
        }
    }

    pub(crate) async fn resolve(
        &mut self,
        dezoomer: &mut dyn Dezoomer,
        uri: &str,
    ) -> Result<Images, DezoomerError> {
        let mut input = DezoomerInput {
            uri: uri.to_string(),
            contents: PageContents::Unknown,
        };

        loop {
            match dezoomer.images(&input) {
                Ok(images) => return Ok(images),
                Err(DezoomerError::NeedsData { uri }) => {
                    let contents = if let Some(contents) = self.cache.get(&uri) {
                        debug!("Using cached metadata for {uri}");
                        contents.clone()
                    } else {
                        let contents = fetch_uri(&uri, self.http).await.map_err(|error| {
                            DezoomerError::DownloadError {
                                msg: error.to_string(),
                            }
                        })?;
                        self.cache.insert(uri.clone(), contents.clone());
                        contents
                    };
                    input = DezoomerInput {
                        uri,
                        contents: PageContents::Success(contents),
                    };
                }
                Err(error) => return Err(error),
            }
        }
    }
}

/// Reorder dezoomers to prioritize those most likely to handle the given URL
#[must_use]
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
        debug!("URL '{url}' appears to match '{preferred_name}' dezoomer, prioritizing it");

        // Move the preferred dezoomer to the front
        let preferred_idx = dezoomers.iter().position(|d| d.name() == preferred_name);
        if let Some(idx) = preferred_idx {
            let preferred = dezoomers.remove(idx);
            dezoomers.insert(0, preferred);
        }
    }

    dezoomers
}

#[must_use]
pub fn all_dezoomers(include_generic: bool) -> Vec<Box<dyn Dezoomer>> {
    let mut dezoomers: Vec<Box<dyn Dezoomer>> = vec![
        Box::<crate::custom_yaml::CustomDezoomer>::default(),
        Box::<crate::google_arts_and_culture::GAPDezoomer>::default(),
        Box::<crate::zoomify::ZoomifyDezoomer>::default(),
        Box::<crate::iiif::IIIF>::default(),
        Box::<crate::dzi::DziDezoomer>::default(),
        Box::<crate::generic::GenericDezoomer>::default(),
        Box::<crate::krpano::KrpanoDezoomer>::default(),
        Box::<crate::iipimage::IIPImage>::default(),
        Box::<crate::nypl::NYPLImage>::default(),
        Box::<crate::bulk_text::BulkTextDezoomer>::default(),
    ];
    if include_generic {
        dezoomers.push(Box::<AutoDezoomer>::default());
    }
    dezoomers
}

struct Candidate {
    dezoomer: Box<dyn Dezoomer>,
    waiting_for: Option<String>,
}

pub struct AutoDezoomer {
    candidates: Vec<Candidate>,
    errors: Vec<(&'static str, DezoomerError)>,
    needs_uris: Vec<String>,
    initialized: bool,
}

impl Default for AutoDezoomer {
    fn default() -> Self {
        AutoDezoomer {
            candidates: all_dezoomers(false)
                .into_iter()
                .map(|dezoomer| Candidate {
                    dezoomer,
                    waiting_for: None,
                })
                .collect(),
            errors: vec![],
            needs_uris: vec![],
            initialized: false,
        }
    }
}

impl AutoDezoomer {
    fn initialize(&mut self, url: &str) {
        if self.initialized {
            return;
        }
        debug!("Prioritizing dezoomers for URL: {url}");
        let dezoomers = std::mem::take(&mut self.candidates)
            .into_iter()
            .map(|candidate| candidate.dezoomer)
            .collect();
        self.candidates = prioritize_dezoomers_for_url(url, dezoomers)
            .into_iter()
            .map(|dezoomer| Candidate {
                dezoomer,
                waiting_for: None,
            })
            .collect();
        self.initialized = true;
    }

    fn next_needed_uri(&mut self) -> Option<String> {
        while let Some(uri) = self.needs_uris.pop() {
            if self
                .candidates
                .iter()
                .any(|candidate| candidate.waiting_for.as_deref() == Some(uri.as_str()))
            {
                return Some(uri);
            }
        }
        None
    }
}

impl Dezoomer for AutoDezoomer {
    fn name(&self) -> &'static str {
        "auto"
    }

    fn images(&mut self, data: &DezoomerInput) -> Result<Images, DezoomerError> {
        let initial_call = !self.initialized;
        self.initialize(&data.uri);

        // TO DO: Use drain_filter when it is stabilized
        let mut i = 0;
        while i != self.candidates.len() {
            let candidate = &mut self.candidates[i];
            if !initial_call && candidate.waiting_for.as_deref() != Some(data.uri.as_str()) {
                i += 1;
                continue;
            }

            candidate.waiting_for = None;
            let keep = match candidate.dezoomer.images(data) {
                Ok(result) => {
                    debug!(
                        "dezoomer '{}' successfully processed the input",
                        candidate.dezoomer.name()
                    );
                    return Ok(result);
                }
                Err(DezoomerError::NeedsData { uri }) => {
                    debug!(
                        "dezoomer '{}' requested to load {}",
                        candidate.dezoomer.name(),
                        uri
                    );
                    if !self.needs_uris.contains(&uri) {
                        self.needs_uris.push(uri.clone());
                    }
                    candidate.waiting_for = Some(uri);
                    true
                }
                Err(e) => {
                    debug!(
                        "{} cannot process this image: {}",
                        candidate.dezoomer.name(),
                        e
                    );
                    self.errors.push((candidate.dezoomer.name(), e));
                    false
                }
            };
            if keep {
                i += 1;
            } else {
                self.candidates.remove(i);
            }
        }
        if let Some(uri) = self.next_needed_uri() {
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
        for (dezoomer_name, err) in &self.0 {
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
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use super::*;

    enum Step {
        Request(&'static str),
        Reject,
        Succeed,
    }

    struct ScriptedDezoomer {
        name: &'static str,
        steps: VecDeque<Step>,
        seen_uris: Rc<RefCell<Vec<String>>>,
    }

    impl ScriptedDezoomer {
        fn new(
            name: &'static str,
            steps: impl IntoIterator<Item = Step>,
        ) -> (Self, Rc<RefCell<Vec<String>>>) {
            let seen_uris = Rc::new(RefCell::new(Vec::new()));
            (
                Self {
                    name,
                    steps: steps.into_iter().collect(),
                    seen_uris: Rc::clone(&seen_uris),
                },
                seen_uris,
            )
        }
    }

    impl Dezoomer for ScriptedDezoomer {
        fn name(&self) -> &'static str {
            self.name
        }

        fn images(&mut self, data: &DezoomerInput) -> Result<Images, DezoomerError> {
            self.seen_uris.borrow_mut().push(data.uri.clone());
            match self.steps.pop_front().expect("unexpected dezoomer call") {
                Step::Request(uri) => Err(DezoomerError::NeedsData { uri: uri.into() }),
                Step::Reject => Err(self.wrong_dezoomer()),
                Step::Succeed => Ok(Images::default()),
            }
        }
    }

    fn auto_with(dezoomers: Vec<Box<dyn Dezoomer>>) -> AutoDezoomer {
        AutoDezoomer {
            candidates: dezoomers
                .into_iter()
                .map(|dezoomer| Candidate {
                    dezoomer,
                    waiting_for: None,
                })
                .collect(),
            errors: Vec::new(),
            needs_uris: Vec::new(),
            initialized: false,
        }
    }

    fn input(uri: &str, contents: PageContents) -> DezoomerInput {
        DezoomerInput {
            uri: uri.into(),
            contents,
        }
    }

    fn needed_uri(result: Result<Images, DezoomerError>) -> String {
        match result {
            Err(DezoomerError::NeedsData { uri }) => uri,
            other => panic!("expected NeedsData, got {other:?}"),
        }
    }

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

    #[test]
    fn shared_followup_is_delivered_to_all_waiting_candidates() {
        let (first, first_seen) =
            ScriptedDezoomer::new("first", [Step::Request("shared"), Step::Reject]);
        let (second, second_seen) =
            ScriptedDezoomer::new("second", [Step::Request("shared"), Step::Succeed]);
        let mut auto = auto_with(vec![Box::new(first), Box::new(second)]);

        assert_eq!(
            needed_uri(auto.images(&input("root", PageContents::Unknown))),
            "shared"
        );
        auto.images(&input(
            "shared",
            PageContents::Success(b"metadata".to_vec()),
        ))
        .expect("the second candidate should accept the shared response");

        assert_eq!(&*first_seen.borrow(), &["root", "shared"]);
        assert_eq!(&*second_seen.borrow(), &["root", "shared"]);
    }

    #[test]
    fn followups_are_delivered_only_to_their_requesters() {
        let (first, first_seen) =
            ScriptedDezoomer::new("first", [Step::Request("first-uri"), Step::Succeed]);
        let (second, second_seen) =
            ScriptedDezoomer::new("second", [Step::Request("second-uri"), Step::Reject]);
        let mut auto = auto_with(vec![Box::new(first), Box::new(second)]);

        assert_eq!(
            needed_uri(auto.images(&input("root", PageContents::Unknown))),
            "second-uri"
        );
        assert_eq!(
            needed_uri(auto.images(&input(
                "second-uri",
                PageContents::Success(b"second".to_vec())
            ))),
            "first-uri"
        );
        auto.images(&input(
            "first-uri",
            PageContents::Success(b"first".to_vec()),
        ))
        .expect("the first candidate should remain active");

        assert_eq!(&*first_seen.borrow(), &["root", "first-uri"]);
        assert_eq!(&*second_seen.borrow(), &["root", "second-uri"]);
    }

    struct RepeatingDezoomer {
        metadata_uri: String,
        calls: usize,
    }

    impl Dezoomer for RepeatingDezoomer {
        fn name(&self) -> &'static str {
            "repeating"
        }

        fn images(&mut self, data: &DezoomerInput) -> Result<Images, DezoomerError> {
            self.calls += 1;
            match self.calls {
                1 => Err(DezoomerError::NeedsData {
                    uri: self.metadata_uri.clone(),
                }),
                2 => {
                    assert_eq!(data.with_contents()?.contents, b"metadata");
                    std::fs::remove_file(&self.metadata_uri).unwrap();
                    Err(DezoomerError::NeedsData {
                        uri: self.metadata_uri.clone(),
                    })
                }
                3 => {
                    assert_eq!(data.with_contents()?.contents, b"metadata");
                    Ok(Images::default())
                }
                _ => panic!("unexpected dezoomer call"),
            }
        }
    }

    #[tokio::test]
    async fn metadata_resolver_fetches_each_uri_once() {
        use std::io::Write;

        let mut metadata = tempfile::NamedTempFile::new().unwrap();
        metadata.write_all(b"metadata").unwrap();
        let metadata_uri = metadata.path().to_string_lossy().into_owned();
        let mut dezoomer = RepeatingDezoomer {
            metadata_uri,
            calls: 0,
        };
        let http = reqwest::Client::new();
        let mut resolver = MetadataResolver::new(&http);

        resolver
            .resolve(&mut dezoomer, "root")
            .await
            .expect("the cached response should satisfy the repeated request");
        assert_eq!(dezoomer.calls, 3);
    }
}
