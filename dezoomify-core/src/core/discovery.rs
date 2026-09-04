//! Pure, resumable discovery orchestration.
//!
//! A discovery program declares how acquired resources are matched and what
//! resource to follow next. The application owns acquisition and feeds each
//! outcome to [`DiscoveryOperation`].

use std::borrow::Cow;
use std::fmt;

use super::model::{ImageCatalog, Request};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RequestId(pub usize);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceNeed {
    pub id: RequestId,
    pub request: Request,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceResponse {
    pub id: RequestId,
    pub bytes: Vec<u8>,
    final_uri: Option<String>,
}
impl ResourceResponse {
    #[must_use]
    pub fn new(id: RequestId, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            id,
            bytes: bytes.into(),
            final_uri: None,
        }
    }

    /// Set the URI reached after the host followed redirects.
    #[must_use]
    pub fn with_final_uri(mut self, uri: impl Into<String>) -> Self {
        self.final_uri = Some(uri.into());
        self
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceFailure {
    pub id: RequestId,
    pub message: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum ResourceOutcome {
    Response(ResourceResponse),
    Failure(ResourceFailure),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryLimits {
    pub transitions: usize,
    pub resources: usize,
    pub retained_bytes: usize,
}

impl Default for DiscoveryLimits {
    fn default() -> Self {
        Self {
            transitions: 10_000,
            resources: 256,
            retained_bytes: 64 * 1024 * 1024,
        }
    }
}

pub enum DiscoveryStep {
    Follow(Request),
    Complete(ImageCatalog),
}
#[derive(Clone, Copy, Debug)]
pub struct DiscoveryResource<'a> {
    request: &'a Request,
    bytes: &'a [u8],
    final_uri: &'a str,
}

impl<'a> DiscoveryResource<'a> {
    #[must_use]
    pub const fn uri(self) -> &'a str {
        self.request.uri.as_str()
    }
    /// The URI after host-level redirects, or [`Self::uri`] when unavailable.
    #[must_use]
    pub const fn final_uri(self) -> &'a str {
        self.final_uri
    }
    #[must_use]
    pub fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    /// Decode resource bytes as UTF-8, replacing malformed sequences.
    #[must_use]
    pub fn text_lossy(self) -> Cow<'a, str> {
        String::from_utf8_lossy(self.bytes)
    }
}
pub struct DiscoveryContext<'a> {
    history_ids: &'a [RequestId],
    requests: &'a [ResourceRecord],
}

impl<'a> DiscoveryContext<'a> {
    #[must_use]
    pub fn resources(&self) -> impl DoubleEndedIterator<Item = DiscoveryResource<'a>> + '_ {
        self.history_ids
            .iter()
            .filter_map(|id| self.requests.get(id.0))
            .filter_map(|record| match record.outcome.as_ref()? {
                ResourceOutcome::Response(response) => Some(DiscoveryResource {
                    request: &record.request,
                    bytes: &response.bytes,
                    final_uri: response.final_uri.as_deref().unwrap_or(&record.request.uri),
                }),
                ResourceOutcome::Failure(_) => None,
            })
    }
    #[must_use]
    pub fn has_visited(&self, uri: &str) -> bool {
        self.history_ids.iter().any(|id| {
            self.requests.get(id.0).is_some_and(|record| {
                record.request.uri == uri
                    || matches!(
                        record.outcome.as_ref(),
                        Some(ResourceOutcome::Response(response))
                            if response.final_uri.as_deref() == Some(uri)
                    )
            })
        })
    }
}
type RouteHandler = for<'a> fn(
    &DiscoveryContext<'a>,
    DiscoveryResource<'a>,
) -> Result<DiscoveryStep, DiscoveryError>;
type CatalogExtractor = fn(&str, &[u8]) -> Result<ImageCatalog, DiscoveryError>;
type FailureHandler = for<'a> fn(
    &DiscoveryContext<'a>,
    &'a Request,
    &'a ResourceFailure,
) -> Result<DiscoveryStep, DiscoveryError>;
type UrlMapper = fn(&str) -> Result<Request, DiscoveryError>;
type UrlPredicate = fn(&str) -> bool;
type ContentPredicate = fn(&[u8]) -> bool;

#[derive(Clone, Copy, Debug)]
pub enum DiscoveryMatch {
    Any,
    UrlSuffix(&'static str),
    UrlPredicate(UrlPredicate),
    ContentPredicate(ContentPredicate),
}

impl DiscoveryMatch {
    #[must_use]
    pub const fn then(self, handler: RouteHandler) -> DiscoveryRoute {
        self.route(RouteAction::Then(handler))
    }
    #[must_use]
    pub const fn extract(self, extractor: CatalogExtractor) -> DiscoveryRoute {
        self.route(RouteAction::Extract(extractor))
    }
    #[must_use]
    pub const fn map_url(self, mapper: UrlMapper) -> DiscoveryRoute {
        self.route(RouteAction::MapUrl(mapper))
    }
    const fn route(self, handler: RouteAction) -> DiscoveryRoute {
        DiscoveryRoute {
            matcher: self,
            handler,
        }
    }

    fn matches(self, uri: &str, bytes: Option<&[u8]>) -> bool {
        match self {
            Self::Any => true,
            Self::UrlSuffix(suffix) => uri
                .split(['?', '#'])
                .next()
                .unwrap_or(uri)
                .ends_with(suffix),
            Self::UrlPredicate(predicate) => predicate(uri),
            Self::ContentPredicate(predicate) => bytes.is_some_and(predicate),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DiscoveryRoute {
    matcher: DiscoveryMatch,
    handler: RouteAction,
}

#[derive(Clone, Copy, Debug)]
enum RouteAction {
    Then(RouteHandler),
    Extract(CatalogExtractor),
    MapUrl(UrlMapper),
}

fn dispatch_resource(
    routes: &[DiscoveryRoute],
    context: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
    unmatched: &str,
) -> Result<DiscoveryStep, DiscoveryError> {
    for route in routes {
        let matches_final = route
            .matcher
            .matches(resource.final_uri(), Some(resource.bytes()));
        let matches_requested = route
            .matcher
            .matches(resource.uri(), Some(resource.bytes()));
        if !matches_final && !matches_requested {
            continue;
        }
        return match route.handler {
            RouteAction::Then(handler) => handler(context, resource),
            RouteAction::Extract(extractor) => {
                extractor(resource.final_uri(), resource.bytes()).map(DiscoveryStep::Complete)
            }
            RouteAction::MapUrl(_) => continue,
        };
    }
    Err(DiscoveryError::Session(unmatched.into()))
}

fn map_url(routes: &[DiscoveryRoute], request: Request) -> Result<Request, DiscoveryError> {
    for route in routes {
        if let RouteAction::MapUrl(mapper) = route.handler
            && route.matcher.matches(&request.uri, None)
        {
            return mapper(&request.uri);
        }
    }
    Ok(request)
}

#[derive(Clone, Copy, Debug)]
enum DiscoveryProgram {
    Immediate(fn(&str) -> Result<ImageCatalog, DiscoveryError>),
    Rules(&'static [DiscoveryRoute], Option<FailureHandler>),
}

#[derive(Clone, Copy, Debug)]
pub struct DezoomerSpec {
    name: &'static str,
    recognize: fn(&str) -> bool,
    rejection: &'static str,
    prefer: fn(&str) -> bool,
    program: DiscoveryProgram,
}

impl DezoomerSpec {
    #[must_use]
    pub const fn new(name: &'static str, routes: &'static [DiscoveryRoute]) -> Self {
        Self::from_program(name, DiscoveryProgram::Rules(routes, None))
    }
    #[must_use]
    pub const fn immediate(
        name: &'static str,
        complete: fn(&str) -> Result<ImageCatalog, DiscoveryError>,
    ) -> Self {
        Self::from_program(name, DiscoveryProgram::Immediate(complete))
    }
    #[must_use]
    pub const fn on_failure(mut self, handler: FailureHandler) -> Self {
        let DiscoveryProgram::Rules(routes, ..) = self.program else {
            panic!("an immediate dezoomer cannot handle resource failures");
        };
        self.program = DiscoveryProgram::Rules(routes, Some(handler));
        self
    }
    const fn from_program(name: &'static str, program: DiscoveryProgram) -> Self {
        Self {
            name,
            recognize: |_| true,
            rejection: "input not recognized",
            prefer: |_| false,
            program,
        }
    }
    #[must_use]
    pub const fn recognizing(
        mut self,
        recognize: fn(&str) -> bool,
        rejection: &'static str,
    ) -> Self {
        self.recognize = recognize;
        self.rejection = rejection;
        self
    }
    #[must_use]
    pub const fn preferring(mut self, prefer: fn(&str) -> bool) -> Self {
        self.prefer = prefer;
        self
    }
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    #[must_use]
    pub fn prefers(&self, uri: &str) -> bool {
        (self.prefer)(uri)
    }
}

impl PartialEq for DezoomerSpec {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscoveryError {
    UnknownRequest(RequestId),
    RequestAlreadyProvided(RequestId),
    NoCandidateAccepted { diagnostics: Vec<(String, String)> },
    NotComplete,
    Session(String),
    TransitionLimitExceeded,
    MetadataSizeLimitExceeded,
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownRequest(id) => write!(f, "unknown discovery request {}", id.0),
            Self::RequestAlreadyProvided(id) => write!(f, "request {} was already supplied", id.0),
            Self::NoCandidateAccepted { diagnostics } => {
                f.write_str("no discovery candidate accepted the input")?;
                for (id, diagnostic) in diagnostics {
                    write!(f, "\n - {id}: {diagnostic}")?;
                }
                Ok(())
            }
            Self::NotComplete => f.write_str("discovery is not complete"),
            Self::Session(message) => f.write_str(message),
            Self::TransitionLimitExceeded => f.write_str("discovery transition limit exceeded"),
            Self::MetadataSizeLimitExceeded => {
                f.write_str("discovery metadata size limit exceeded")
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

#[derive(Clone, Copy)]
enum CandidateState {
    New,
    Waiting(RequestId),
    Rejected,
}

struct Candidate {
    spec: DezoomerSpec,
    state: CandidateState,
    history: Vec<RequestId>,
}

struct ResourceRecord {
    request: Request,
    outcome: Option<ResourceOutcome>,
}

pub struct DiscoveryOperation {
    input: String,
    candidates: Vec<Candidate>,
    requests: Vec<ResourceRecord>,
    diagnostics: Vec<(String, String)>,
    catalog: Option<ImageCatalog>,
    transitions: usize,
    retained_bytes: usize,
    limits: DiscoveryLimits,
}

impl DiscoveryOperation {
    pub(crate) fn new(input: String, specs: &[DezoomerSpec], limits: DiscoveryLimits) -> Self {
        let candidates = specs
            .iter()
            .map(|&spec| Candidate {
                spec,
                state: CandidateState::New,
                history: Vec::new(),
            })
            .collect();
        Self {
            input,
            candidates,
            requests: Vec::new(),
            diagnostics: Vec::new(),
            catalog: None,
            transitions: 0,
            retained_bytes: 0,
            limits,
        }
    }
    fn resource(&self, id: RequestId) -> Option<&ResourceRecord> {
        self.requests.get(id.0)
    }
    fn ready(&self, state: CandidateState) -> bool {
        matches!(state, CandidateState::New)
            || matches!(state, CandidateState::Waiting(id) if self
                .resource(id)
                .is_some_and(|resource| resource.outcome.is_some()))
    }
    pub fn missing_resources(&mut self) -> Result<Vec<ResourceNeed>, DiscoveryError> {
        self.drive()?;
        Ok(if self.catalog.is_none() {
            self.outstanding_needs().collect()
        } else {
            Vec::new()
        })
    }
    pub fn next_priority_need(&mut self) -> Result<Option<ResourceNeed>, DiscoveryError> {
        self.drive()?;
        if self.catalog.is_some() {
            return Ok(None);
        }
        for candidate in &self.candidates {
            if let CandidateState::Waiting(id) = candidate.state
                && let Some(resource) = self.resource(id)
                && resource.outcome.is_none()
            {
                return Ok(Some(ResourceNeed {
                    id,
                    request: resource.request.clone(),
                }));
            }
        }
        Ok(self.outstanding_needs().next())
    }
    fn outstanding_needs(&self) -> impl Iterator<Item = ResourceNeed> + '_ {
        self.requests
            .iter()
            .enumerate()
            .filter(|(_, resource)| resource.outcome.is_none())
            .map(|(index, resource)| ResourceNeed {
                id: RequestId(index),
                request: resource.request.clone(),
            })
    }
    pub fn provide(&mut self, response: ResourceResponse) -> Result<(), DiscoveryError> {
        self.provide_outcome(response.id, ResourceOutcome::Response(response))
    }
    pub fn provide_failure(&mut self, failure: ResourceFailure) -> Result<(), DiscoveryError> {
        self.provide_outcome(failure.id, ResourceOutcome::Failure(failure))
    }
    fn provide_outcome(
        &mut self,
        id: RequestId,
        outcome: ResourceOutcome,
    ) -> Result<(), DiscoveryError> {
        let Some(resource) = self.requests.get(id.0) else {
            return Err(DiscoveryError::UnknownRequest(id));
        };
        if resource.outcome.is_some() {
            return Err(DiscoveryError::RequestAlreadyProvided(id));
        }
        if let ResourceOutcome::Response(response) = &outcome {
            self.retained_bytes = self
                .retained_bytes
                .checked_add(response.bytes.len())
                .filter(|total| *total <= self.limits.retained_bytes)
                .ok_or(DiscoveryError::MetadataSizeLimitExceeded)?;
        }
        self.requests[id.0].outcome = Some(outcome);
        self.drive()
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.catalog.is_some()
    }

    pub fn finish(mut self) -> Result<ImageCatalog, DiscoveryError> {
        self.drive()?;
        self.catalog.take().ok_or(DiscoveryError::NotComplete)
    }
    fn drive(&mut self) -> Result<(), DiscoveryError> {
        while self.catalog.is_none() {
            let Some(index) = self
                .candidates
                .iter()
                .position(|candidate| self.ready(candidate.state))
            else {
                let pending = self.requests.iter().any(|r| r.outcome.is_none())
                    || self
                        .candidates
                        .iter()
                        .any(|c| !matches!(c.state, CandidateState::Rejected));
                if pending {
                    return Ok(());
                }
                return Err(DiscoveryError::NoCandidateAccepted {
                    diagnostics: self.diagnostics.clone(),
                });
            };
            self.transitions += 1;
            if self.transitions > self.limits.transitions {
                return Err(DiscoveryError::TransitionLimitExceeded);
            }
            let result = self
                .advance_candidate(index)
                .and_then(|step| self.apply_step(index, step));
            match result {
                Err(DiscoveryError::Session(message)) => self.reject_candidate(index, message),
                result => result?,
            }
        }
        Ok(())
    }

    fn advance_candidate(&mut self, index: usize) -> Result<DiscoveryStep, DiscoveryError> {
        let candidate = &self.candidates[index];
        if matches!(candidate.state, CandidateState::New) {
            if !(candidate.spec.recognize)(&self.input) {
                return Err(DiscoveryError::Session(candidate.spec.rejection.into()));
            }
            return match candidate.spec.program {
                DiscoveryProgram::Immediate(complete) => {
                    complete(&self.input).map(DiscoveryStep::Complete)
                }
                DiscoveryProgram::Rules(..) => {
                    Ok(DiscoveryStep::Follow(Request::new(self.input.clone())))
                }
            };
        }

        let CandidateState::Waiting(id) = candidate.state else {
            unreachable!("rejected candidates are not driven");
        };
        let DiscoveryProgram::Rules(routes, on_failure) = candidate.spec.program else {
            unreachable!("immediate dezoomers never follow resources");
        };
        let resource = self.resource(id).expect("ready request exists");
        let request = &resource.request;
        let previous_history = &candidate.history[..candidate.history.len() - 1];
        let context = DiscoveryContext {
            history_ids: previous_history,
            requests: &self.requests,
        };
        match resource
            .outcome
            .as_ref()
            .expect("ready candidate has an outcome")
        {
            ResourceOutcome::Response(response) => dispatch_resource(
                routes,
                &context,
                DiscoveryResource {
                    request,
                    bytes: &response.bytes,
                    final_uri: response.final_uri.as_deref().unwrap_or(&request.uri),
                },
                "resource did not match any discovery route",
            ),
            ResourceOutcome::Failure(failure) => on_failure.map_or_else(
                || Err(DiscoveryError::Session(failure.message.clone())),
                |handler| handler(&context, request, failure),
            ),
        }
    }

    fn apply_step(&mut self, index: usize, step: DiscoveryStep) -> Result<(), DiscoveryError> {
        match step {
            DiscoveryStep::Follow(request) => {
                let request = match self.candidates[index].spec.program {
                    DiscoveryProgram::Rules(routes, ..) => map_url(routes, request)?,
                    DiscoveryProgram::Immediate(_) => request,
                };
                let Some(id) = self.register_request(request) else {
                    self.reject_candidate(index, "discovery resource limit exceeded".into());
                    return Ok(());
                };
                if self.candidates[index].history.contains(&id) {
                    self.reject_candidate(
                        index,
                        "discovery followed the same resource twice".into(),
                    );
                } else {
                    self.candidates[index].history.push(id);
                    self.candidates[index].state = CandidateState::Waiting(id);
                }
            }
            DiscoveryStep::Complete(catalog) => {
                self.catalog = Some(
                    catalog
                        .normalize()
                        .map_err(|error| DiscoveryError::Session(error.to_string()))?,
                );
            }
        }
        Ok(())
    }

    fn reject_candidate(&mut self, index: usize, diagnostic: String) {
        let id = self.candidates[index].spec.name.to_owned();
        self.diagnostics.push((id, diagnostic));
        self.candidates[index].state = CandidateState::Rejected;
    }

    fn register_request(&mut self, request: Request) -> Option<RequestId> {
        if let Some(index) = self
            .requests
            .iter()
            .position(|resource| resource.request == request)
        {
            return Some(RequestId(index));
        }
        if self.requests.len() >= self.limits.resources {
            return None;
        }
        let id = RequestId(self.requests.len());
        self.requests.push(ResourceRecord {
            request,
            outcome: None,
        });
        Some(id)
    }
}

#[cfg(test)]
#[allow(clippy::unnecessary_wraps)]
mod tests {
    use super::*;
    use crate::core::model::{CatalogEntry, ImageDescriptor};
    use crate::core::registry::Registry;

    fn catalog(_: &str, _: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
        Ok(ImageCatalog::default())
    }

    fn final_uri_catalog(uri: &str, _: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
        Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
            title: Some(uri.into()),
            ..Default::default()
        })]))
    }

    const FINAL_URI: &[DiscoveryRoute] =
        &[DiscoveryMatch::UrlSuffix("/redirect").extract(final_uri_catalog)];

    fn reject(
        _: &DiscoveryContext<'_>,
        _: DiscoveryResource<'_>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        Err(DiscoveryError::Session("wrong format".into()))
    }

    const COMPLETE: &[DiscoveryRoute] = &[DiscoveryMatch::Any.extract(catalog)];
    const REJECT: &[DiscoveryRoute] = &[DiscoveryMatch::Any.then(reject)];

    fn provide(operation: &mut DiscoveryOperation, bytes: &[u8]) {
        let need = operation.missing_resources().unwrap().pop().unwrap();
        operation
            .provide(ResourceResponse::new(need.id, bytes))
            .unwrap();
    }

    #[test]
    fn input_acquisition_is_implicit_and_extractors_receive_it() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::new("test", COMPLETE));
        let mut operation = registry.start("memory://metadata");
        let need = operation.missing_resources().unwrap().pop().unwrap();
        assert_eq!(need.request.uri, "memory://metadata");
        operation
            .provide(ResourceResponse::new(need.id, b"metadata"))
            .unwrap();
        assert!(operation.finish().unwrap().is_empty());
    }

    #[test]
    fn extractors_receive_the_redirect_target_uri() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::new("final-uri", FINAL_URI));
        let mut operation = registry.start("https://example.test/redirect");
        let need = operation.missing_resources().unwrap().pop().unwrap();
        operation
            .provide(
                ResourceResponse::new(need.id, b"metadata")
                    .with_final_uri("https://cdn.example.test/info.xml"),
            )
            .unwrap();
        let catalog = operation.finish().unwrap();
        let [CatalogEntry::Ready(image)] = catalog.entries() else {
            panic!("expected one ready image")
        };
        assert_eq!(
            image.title.as_deref(),
            Some("https://cdn.example.test/info.xml")
        );
    }

    fn text_catalog(
        _: &DiscoveryContext<'_>,
        resource: DiscoveryResource<'_>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        assert_eq!(resource.text_lossy(), "metadata\u{fffd}");
        Ok(DiscoveryStep::Complete(ImageCatalog::default()))
    }

    const TEXT: &[DiscoveryRoute] = &[DiscoveryMatch::Any.then(text_catalog)];

    #[test]
    fn resources_expose_lossy_text() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::new("text", TEXT));
        let mut operation = registry.start("memory://metadata");
        provide(&mut operation, b"metadata\xff");
        assert!(operation.finish().unwrap().is_empty());
    }

    fn is_tile(uri: &str) -> bool {
        uri.ends_with("/tile.jpg")
    }

    fn tile_metadata(uri: &str) -> Result<Request, DiscoveryError> {
        Ok(Request::new(uri.replace("/tile.jpg", "/metadata")))
    }

    const MAPPED: &[DiscoveryRoute] = &[
        DiscoveryMatch::UrlPredicate(is_tile).map_url(tile_metadata),
        DiscoveryMatch::UrlSuffix("/metadata").extract(catalog),
    ];

    #[test]
    fn url_mapping_happens_before_acquisition() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::new("mapped", MAPPED));
        let mut operation = registry.start("memory://image/tile.jpg");
        assert_eq!(
            operation.next_priority_need().unwrap().unwrap().request.uri,
            "memory://image/metadata"
        );
    }

    fn follow_tiles(
        context: &DiscoveryContext<'_>,
        resource: DiscoveryResource<'_>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        assert!(context.resources().next().is_none());
        Ok(DiscoveryStep::Follow(
            Request::new(format!("{}/tiles", resource.uri())).with_header("X-Test", "preserved"),
        ))
    }

    fn finish_chain(
        context: &DiscoveryContext<'_>,
        resource: DiscoveryResource<'_>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        assert!(resource.uri().ends_with("/tiles"));
        assert_eq!(context.resources().count(), 1);
        Ok(DiscoveryStep::Complete(ImageCatalog::default()))
    }

    const CHAIN: &[DiscoveryRoute] = &[
        DiscoveryMatch::UrlSuffix("/metadata").then(follow_tiles),
        DiscoveryMatch::UrlSuffix("/tiles").then(finish_chain),
    ];

    #[test]
    fn followed_resources_are_redispatched_with_history() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::new("chain", CHAIN));
        let mut operation = registry.start("memory://image/metadata");
        provide(&mut operation, b"metadata");
        let need = operation.next_priority_need().unwrap().unwrap();
        assert_eq!(need.request.uri, "memory://image/metadata/tiles");
        assert_eq!(
            need.request.headers.get("X-Test").map(String::as_str),
            Some("preserved")
        );
        provide(&mut operation, b"tiles");
        assert!(operation.finish().unwrap().is_empty());
    }

    #[test]
    fn identical_requests_are_fanned_out_and_parser_errors_try_the_next_candidate() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::new("reject", REJECT));
        registry.register(DezoomerSpec::new("accept", COMPLETE));
        let mut operation = registry.start("memory://shared");
        assert_eq!(operation.missing_resources().unwrap().len(), 1);
        provide(&mut operation, b"metadata");
        assert!(operation.is_complete());
        assert!(operation.finish().unwrap().is_empty());
    }

    fn follow_a(
        context: &DiscoveryContext<'_>,
        _: DiscoveryResource<'_>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        assert!(context.resources().next().is_none());
        Ok(DiscoveryStep::Follow(Request::new("memory://a")))
    }

    fn follow_b(
        context: &DiscoveryContext<'_>,
        _: DiscoveryResource<'_>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        assert!(context.resources().next().is_none());
        Ok(DiscoveryStep::Follow(Request::new("memory://b")))
    }

    const HISTORY_A: &[DiscoveryRoute] = &[DiscoveryMatch::Any.then(follow_a)];
    const HISTORY_B: &[DiscoveryRoute] = &[DiscoveryMatch::Any.then(follow_b)];

    #[test]
    fn history_is_candidate_local_when_requests_are_shared() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::new("a", HISTORY_A));
        registry.register(DezoomerSpec::new("b", HISTORY_B));
        let mut operation = registry.start("memory://shared");
        let shared = operation.missing_resources().unwrap().pop().unwrap();
        operation
            .provide(ResourceResponse::new(shared.id, b"shared"))
            .unwrap();
        assert_eq!(operation.candidates[0].history[0], shared.id);
        assert_eq!(operation.candidates[1].history[0], shared.id);
        assert_ne!(
            operation.candidates[0].history[1],
            operation.candidates[1].history[1]
        );
    }

    fn recover(
        _: &DiscoveryContext<'_>,
        request: &Request,
        failure: &ResourceFailure,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        assert_eq!(request.uri, "memory://failure");
        assert_eq!(failure.message, "expected");
        Ok(DiscoveryStep::Complete(ImageCatalog::default()))
    }

    #[test]
    fn failure_handlers_choose_the_next_action() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::new("failure", COMPLETE).on_failure(recover));
        let mut operation = registry.start("memory://failure");
        let need = operation.missing_resources().unwrap().pop().unwrap();
        operation
            .provide_failure(ResourceFailure {
                id: need.id,
                message: "expected".into(),
            })
            .unwrap();
        assert!(operation.finish().unwrap().is_empty());
    }

    fn repeat(
        _: &DiscoveryContext<'_>,
        resource: DiscoveryResource<'_>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        Ok(DiscoveryStep::Follow(Request::new(resource.uri())))
    }

    const REPEAT: &[DiscoveryRoute] = &[DiscoveryMatch::Any.then(repeat)];

    #[test]
    fn following_the_same_uri_is_rejected() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::new("repeat", REPEAT));
        let mut operation = registry.start("memory://repeat");
        let need = operation.missing_resources().unwrap().pop().unwrap();
        let error = operation
            .provide(ResourceResponse::new(need.id, b"again"))
            .unwrap_err();
        assert!(error.to_string().contains("same resource twice"));
    }

    fn follow_again(
        context: &DiscoveryContext<'_>,
        _: DiscoveryResource<'_>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        Ok(DiscoveryStep::Follow(Request::new(format!(
            "memory://{}",
            context.resources().count()
        ))))
    }

    const LOOP: &[DiscoveryRoute] = &[DiscoveryMatch::Any.then(follow_again)];

    #[test]
    fn operation_limits_are_enforced() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::new("loop", LOOP));
        let mut transitions = registry.start_with_limits(
            "memory://start",
            DiscoveryLimits {
                transitions: 2,
                ..Default::default()
            },
        );
        let need = transitions.missing_resources().unwrap().pop().unwrap();
        transitions
            .provide(ResourceResponse::new(need.id, []))
            .unwrap();
        let need = transitions.missing_resources().unwrap().pop().unwrap();
        let error = transitions
            .provide(ResourceResponse::new(need.id, []))
            .unwrap_err();
        assert_eq!(error, DiscoveryError::TransitionLimitExceeded);

        let mut limited = Registry::new();
        limited.register(DezoomerSpec::new("high", HIGH));
        limited.register(DezoomerSpec::new("low", LOW));
        let mut resources = limited.start_with_limits(
            "memory://root",
            DiscoveryLimits {
                resources: 2,
                ..Default::default()
            },
        );
        let high = resources
            .missing_resources()
            .unwrap()
            .into_iter()
            .find(|need| need.request.uri == "memory://high-1")
            .unwrap();
        assert_eq!(
            resources.provide(ResourceResponse::new(high.id, [])),
            Ok(())
        );
        let low = resources
            .missing_resources()
            .unwrap()
            .into_iter()
            .find(|need| need.request.uri == "memory://low")
            .unwrap();
        resources
            .provide(ResourceResponse::new(low.id, []))
            .unwrap();
        assert!(resources.is_complete());
        assert!(resources.finish().unwrap().is_empty());

        let mut bytes = Registry::new();
        bytes.register(DezoomerSpec::new("bytes", COMPLETE));
        let mut bytes = bytes.start_with_limits(
            "memory://metadata",
            DiscoveryLimits {
                retained_bytes: 1,
                ..Default::default()
            },
        );
        let need = bytes.missing_resources().unwrap().pop().unwrap();
        assert_eq!(
            bytes.provide(ResourceResponse::new(need.id, [0, 1])),
            Err(DiscoveryError::MetadataSizeLimitExceeded)
        );
    }

    fn high_start(_: &str) -> Result<Request, DiscoveryError> {
        Ok(Request::new("memory://high-1"))
    }

    fn low_start(_: &str) -> Result<Request, DiscoveryError> {
        Ok(Request::new("memory://low"))
    }

    fn is_root(uri: &str) -> bool {
        uri == "memory://root"
    }

    fn high_next(
        _: &DiscoveryContext<'_>,
        _: DiscoveryResource<'_>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        Ok(DiscoveryStep::Follow(Request::new("memory://high-2")))
    }

    const HIGH: &[DiscoveryRoute] = &[
        DiscoveryMatch::UrlPredicate(is_root).map_url(high_start),
        DiscoveryMatch::UrlSuffix("high-1").then(high_next),
        DiscoveryMatch::UrlSuffix("high-2").extract(catalog),
    ];
    const LOW: &[DiscoveryRoute] = &[
        DiscoveryMatch::UrlPredicate(is_root).map_url(low_start),
        DiscoveryMatch::Any.extract(catalog),
    ];

    #[test]
    fn priority_stays_depth_first_across_followed_resources() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::new("high", HIGH));
        registry.register(DezoomerSpec::new("low", LOW));
        let mut operation = registry.start("memory://root");
        assert_eq!(operation.missing_resources().unwrap().len(), 2);
        let high = operation.next_priority_need().unwrap().unwrap();
        assert_eq!(high.request.uri, "memory://high-1");
        operation
            .provide(ResourceResponse::new(high.id, []))
            .unwrap();
        assert_eq!(
            operation.next_priority_need().unwrap().unwrap().request.uri,
            "memory://high-2"
        );
    }

    #[test]
    fn diagnostics_are_displayed_one_per_line() {
        let error = DiscoveryError::NoCandidateAccepted {
            diagnostics: vec![
                ("first".into(), "not first".into()),
                ("second".into(), "not second".into()),
            ],
        };
        assert_eq!(
            error.to_string(),
            "no discovery candidate accepted the input\n - first: not first\n - second: not second"
        );
    }
}
