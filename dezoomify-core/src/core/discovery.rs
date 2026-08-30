//! Pure, resumable discovery orchestration.
//!
//! A discovery program declares how acquired resources are matched and what
//! resource to follow next. The application owns acquisition and feeds each
//! outcome to [`DiscoveryOperation`].

use std::collections::BTreeMap;
use std::fmt;

use super::model::{ImageCatalog, Request};

/// Opaque input supplied to a discovery program.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DiscoveryInput {
    pub uri: String,
}

impl From<String> for DiscoveryInput {
    fn from(uri: String) -> Self {
        Self { uri }
    }
}

impl From<&str> for DiscoveryInput {
    fn from(uri: &str) -> Self {
        Self { uri: uri.into() }
    }
}

/// A stable identifier for one requested resource in an operation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RequestId(pub u64);

/// A resource which must be supplied by the application before discovery can
/// continue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceNeed {
    pub id: RequestId,
    pub request: Request,
}

/// Bytes supplied by the application for a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceResponse {
    pub id: RequestId,
    pub bytes: Vec<u8>,
    /// HTTP media type, when the resource provider supplied one.
    pub content_type: Option<String>,
}

impl ResourceResponse {
    #[must_use]
    pub fn new(id: RequestId, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            id,
            bytes: bytes.into(),
            content_type: None,
        }
    }

    #[must_use]
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }
}

/// An application-reported failure to satisfy a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceFailure {
    pub id: RequestId,
    pub message: String,
}

/// The resource result visible to a waiting program.
#[derive(Clone, Debug, Eq, PartialEq)]
enum ResourceOutcome {
    Response(ResourceResponse),
    Failure(ResourceFailure),
}

/// Bounds for a single discovery operation.
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

/// One declarative result yielded by a discovery rule.
pub enum DiscoveryStep {
    /// Continue discovery with another resource described by this request.
    Follow(Request),
    Complete(ImageCatalog),
}

impl DiscoveryStep {
    #[must_use]
    pub fn follow(uri: impl Into<String>) -> Self {
        Self::Follow(Request::new(uri))
    }
}

/// One successfully acquired resource.
#[derive(Clone, Copy, Debug)]
pub struct DiscoveryResource<'a> {
    request: &'a Request,
    response: &'a ResourceResponse,
}

impl<'a> DiscoveryResource<'a> {
    #[must_use]
    pub const fn uri(self) -> &'a str {
        self.request.uri.as_str()
    }

    #[must_use]
    pub fn bytes(self) -> &'a [u8] {
        self.response.bytes.as_slice()
    }

    #[must_use]
    pub fn content_type(self) -> Option<&'a str> {
        self.response.content_type.as_deref()
    }
}

/// One resource acquisition failure delivered to a failure rule.
#[derive(Clone, Copy, Debug)]
pub struct DiscoveryFailure<'a> {
    request: &'a Request,
    failure: &'a ResourceFailure,
}

impl<'a> DiscoveryFailure<'a> {
    #[must_use]
    pub const fn uri(self) -> &'a str {
        self.request.uri.as_str()
    }

    #[must_use]
    pub const fn failure(self) -> &'a ResourceFailure {
        self.failure
    }
}

/// Read-only state supplied to a declarative route handler.
///
/// History is local to this candidate even when requests are deduplicated
/// across candidates.
pub struct DiscoveryContext<'a> {
    history_ids: &'a [RequestId],
    requests: &'a BTreeMap<RequestId, ResourceNeed>,
    outcomes: &'a BTreeMap<RequestId, ResourceOutcome>,
}

impl<'a> DiscoveryContext<'a> {
    /// Earlier successful resources for this candidate, in discovery order.
    #[must_use]
    pub fn resources(&self) -> impl DoubleEndedIterator<Item = DiscoveryResource<'a>> + '_ {
        self.history_ids.iter().filter_map(|id| {
            let request = &self.requests.get(id)?.request;
            let ResourceOutcome::Response(response) = self.outcomes.get(id)? else {
                return None;
            };
            Some(DiscoveryResource { request, response })
        })
    }

    /// Whether this candidate already followed a resource with this URI,
    /// regardless of whether acquisition succeeded.
    #[must_use]
    pub fn has_visited(&self, uri: &str) -> bool {
        self.history_ids.iter().any(|id| {
            self.requests
                .get(id)
                .is_some_and(|resource| resource.request.uri == uri)
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
    DiscoveryFailure<'a>,
) -> Result<DiscoveryStep, DiscoveryError>;
type UrlMapper = fn(&str) -> Result<Request, DiscoveryError>;
type UrlPredicate = fn(&str) -> bool;
type ContentPredicate = fn(&[u8]) -> bool;

/// A predicate for one declarative discovery route.
#[derive(Clone, Copy, Debug)]
pub enum DiscoveryMatch {
    Any,
    UrlSuffix(&'static str),
    UrlPredicate(UrlPredicate),
    MediaTypes(&'static [&'static str]),
    ContentPredicate(ContentPredicate),
}

impl DiscoveryMatch {
    #[must_use]
    pub const fn any() -> Self {
        Self::Any
    }

    #[must_use]
    pub const fn url_suffix(suffix: &'static str) -> Self {
        Self::UrlSuffix(suffix)
    }

    #[must_use]
    pub const fn url_matching(predicate: UrlPredicate) -> Self {
        Self::UrlPredicate(predicate)
    }

    #[must_use]
    pub const fn media_type(media_types: &'static [&'static str]) -> Self {
        Self::MediaTypes(media_types)
    }

    #[must_use]
    pub const fn content_matching(predicate: ContentPredicate) -> Self {
        Self::ContentPredicate(predicate)
    }

    /// Run a multi-resource handler for a matching acquired resource.
    #[must_use]
    pub const fn then(self, handler: RouteHandler) -> DiscoveryRoute {
        DiscoveryRoute {
            matcher: self,
            handler: RouteAction::Then(handler),
        }
    }

    /// Parse a matching resource directly into a catalog.
    #[must_use]
    pub const fn extract(self, extractor: CatalogExtractor) -> DiscoveryRoute {
        DiscoveryRoute {
            matcher: self,
            handler: RouteAction::Extract(extractor),
        }
    }

    /// Rewrite a matching input URI before the core acquires it.
    #[must_use]
    pub const fn map_url(self, mapper: UrlMapper) -> DiscoveryRoute {
        DiscoveryRoute {
            matcher: self,
            handler: RouteAction::MapUrl(mapper),
        }
    }

    fn matches_uri(self, uri: &str) -> bool {
        let path = uri.split(['?', '#']).next().unwrap_or(uri);
        match self {
            Self::Any => true,
            Self::UrlSuffix(suffix) => path.ends_with(suffix),
            Self::UrlPredicate(predicate) => predicate(uri),
            Self::MediaTypes(_) | Self::ContentPredicate(_) => false,
        }
    }

    fn matches_resource(self, resource: DiscoveryResource<'_>) -> bool {
        match self {
            Self::Any => true,
            Self::UrlSuffix(_) | Self::UrlPredicate(_) => self.matches_uri(resource.uri()),
            Self::MediaTypes(types) => resource.content_type().is_some_and(|value| {
                let media_type = value.split(';').next().unwrap_or(value).trim();
                types
                    .iter()
                    .any(|expected| media_type.eq_ignore_ascii_case(expected))
            }),
            Self::ContentPredicate(predicate) => predicate(resource.bytes()),
        }
    }
}

/// An ordered declarative resource matcher and transition handler.
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

impl DiscoveryRoute {
    fn handle(
        self,
        context: &DiscoveryContext<'_>,
        resource: DiscoveryResource<'_>,
    ) -> Option<Result<DiscoveryStep, DiscoveryError>> {
        if !self.matcher.matches_resource(resource) {
            return None;
        }
        Some(match self.handler {
            RouteAction::Then(handler) => handler(context, resource),
            RouteAction::Extract(extractor) => {
                extractor(resource.uri(), resource.bytes()).map(DiscoveryStep::Complete)
            }
            RouteAction::MapUrl(_) => return None,
        })
    }

    fn map_url(self, uri: &str) -> Option<Result<Request, DiscoveryError>> {
        let RouteAction::MapUrl(mapper) = self.handler else {
            return None;
        };
        self.matcher.matches_uri(uri).then(|| mapper(uri))
    }
}

fn dispatch_resource(
    routes: &[DiscoveryRoute],
    context: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
    unmatched: &str,
) -> Result<DiscoveryStep, DiscoveryError> {
    for route in routes {
        let Some(step) = route.handle(context, resource) else {
            continue;
        };
        return step;
    }
    Err(DiscoveryError::Session(unmatched.into()))
}

fn map_url(routes: &[DiscoveryRoute], uri: &str) -> Result<Request, DiscoveryError> {
    for route in routes {
        if let Some(mapped) = route.map_url(uri) {
            return mapped;
        }
    }
    Ok(Request::new(uri))
}

#[derive(Clone, Copy, Debug)]
enum DiscoveryProgram {
    Immediate(fn(&str) -> Result<ImageCatalog, DiscoveryError>),
    Rules {
        routes: &'static [DiscoveryRoute],
        on_failure: Option<FailureHandler>,
    },
}

/// One co-located format declaration.
#[derive(Clone, Copy, Debug)]
pub struct DezoomerSpec {
    name: &'static str,
    recognize: fn(&str) -> bool,
    rejection: &'static str,
    prefer: fn(&str) -> bool,
    program: DiscoveryProgram,
}

impl DezoomerSpec {
    /// Declare ordered rules for resources acquired by the core.
    ///
    /// The core acquires the input URI automatically and dispatches every
    /// successful resource through the same ordered rule list.
    #[must_use]
    pub const fn new(name: &'static str, routes: &'static [DiscoveryRoute]) -> Self {
        Self::from_program(
            name,
            DiscoveryProgram::Rules {
                routes,
                on_failure: None,
            },
        )
    }

    /// Declare a format which completes from its input without a resource.
    #[must_use]
    pub const fn immediate(
        name: &'static str,
        complete: fn(&str) -> Result<ImageCatalog, DiscoveryError>,
    ) -> Self {
        Self::from_program(name, DiscoveryProgram::Immediate(complete))
    }

    /// Handle acquisition failures for formats with alternate resources.
    #[must_use]
    pub const fn on_failure(mut self, handler: FailureHandler) -> Self {
        let DiscoveryProgram::Rules { routes, .. } = self.program else {
            panic!("an immediate dezoomer cannot handle resource failures");
        };
        self.program = DiscoveryProgram::Rules {
            routes,
            on_failure: Some(handler),
        };
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

    /// Add authoritative input recognition and its rejection diagnostic.
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

    /// Prefer this format during automatic discovery when the predicate matches.
    #[must_use]
    pub const fn preferring(mut self, prefer: fn(&str) -> bool) -> Self {
        self.prefer = prefer;
        self
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Whether this format prefers the supplied input URI.
    ///
    /// Custom registry implementations can use this to apply the same ranking
    /// policy as the built-in registry.
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

/// Pure discovery errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscoveryError {
    UnknownRequest(RequestId),
    RequestAlreadyProvided(RequestId),
    NoCandidateAccepted { diagnostics: Vec<(String, String)> },
    NotComplete,
    Session(String),
    TransitionLimitExceeded,
    ResourceLimitExceeded,
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
            Self::ResourceLimitExceeded => f.write_str("discovery resource limit exceeded"),
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

/// A pull-driven operation with no I/O or shared mutable parser state.
pub struct DiscoveryOperation {
    input: DiscoveryInput,
    candidates: Vec<Candidate>,
    requests: BTreeMap<RequestId, ResourceNeed>,
    request_ids: BTreeMap<Request, RequestId>,
    outcomes: BTreeMap<RequestId, ResourceOutcome>,
    diagnostics: Vec<(String, String)>,
    catalog: Option<ImageCatalog>,
    next_request_id: u64,
    transitions: usize,
    retained_bytes: usize,
    limits: DiscoveryLimits,
}

impl DiscoveryOperation {
    pub(crate) fn new(
        input: &DiscoveryInput,
        specs: &[DezoomerSpec],
        limits: DiscoveryLimits,
    ) -> Self {
        let mut candidates = Vec::new();
        for spec in specs {
            candidates.push(Candidate {
                spec: *spec,
                state: CandidateState::New,
                history: Vec::new(),
            });
        }
        // Registry validation and URL ranking supply the canonical candidate
        // order. Keep it intact here; sorting again would discard URL hints.
        Self {
            input: input.clone(),
            candidates,
            requests: BTreeMap::new(),
            request_ids: BTreeMap::new(),
            outcomes: BTreeMap::new(),
            diagnostics: Vec::new(),
            catalog: None,
            next_request_id: 0,
            transitions: 0,
            retained_bytes: 0,
            limits,
        }
    }

    /// Return every outstanding request in stable identifier order.
    ///
    /// # Errors
    ///
    /// Returns an error if a candidate cannot transition.
    pub fn missing_resources(&mut self) -> Result<Vec<ResourceNeed>, DiscoveryError> {
        self.drive()?;
        if self.catalog.is_some() {
            return Ok(Vec::new());
        }
        Ok(self.outstanding_needs().collect())
    }

    /// Return the outstanding resource needed by the highest-priority waiting candidate.
    ///
    /// When candidates wait on different URIs, this picks the resource of the
    /// first waiting candidate in registry order, preserving depth-first
    /// priority. If no candidate is waiting, falls back to the first
    /// outstanding request.
    pub fn next_priority_need(&mut self) -> Result<Option<ResourceNeed>, DiscoveryError> {
        self.drive()?;
        if self.catalog.is_some() {
            return Ok(None);
        }
        for candidate in &self.candidates {
            if let CandidateState::Waiting(id) = candidate.state
                && !self.outcomes.contains_key(&id)
                && let Some(need) = self.requests.get(&id).cloned()
            {
                return Ok(Some(need));
            }
        }
        Ok(self.outstanding_needs().next())
    }

    /// Needs which have no outcome yet, in stable identifier order.
    fn outstanding_needs(&self) -> impl Iterator<Item = ResourceNeed> + '_ {
        self.requests
            .iter()
            .filter(|(id, _)| !self.outcomes.contains_key(id))
            .map(|(_, need)| need.clone())
    }

    /// Supply successfully acquired bytes.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown or already supplied request.
    pub fn provide(&mut self, response: ResourceResponse) -> Result<(), DiscoveryError> {
        self.provide_outcome(response.id, ResourceOutcome::Response(response))
    }

    /// Supply a failed acquisition without imposing retry policy.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown or already supplied request.
    pub fn provide_failure(&mut self, failure: ResourceFailure) -> Result<(), DiscoveryError> {
        self.provide_outcome(failure.id, ResourceOutcome::Failure(failure))
    }

    fn provide_outcome(
        &mut self,
        id: RequestId,
        outcome: ResourceOutcome,
    ) -> Result<(), DiscoveryError> {
        if !self.requests.contains_key(&id) {
            return Err(DiscoveryError::UnknownRequest(id));
        }
        if self.outcomes.contains_key(&id) {
            return Err(DiscoveryError::RequestAlreadyProvided(id));
        }
        if let ResourceOutcome::Response(response) = &outcome {
            self.retained_bytes = self
                .retained_bytes
                .checked_add(response.bytes.len())
                .filter(|total| *total <= self.limits.retained_bytes)
                .ok_or(DiscoveryError::MetadataSizeLimitExceeded)?;
        }
        self.outcomes.insert(id, outcome);
        self.drive()
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.catalog.is_some()
    }

    /// Consume this completed operation.
    ///
    /// # Errors
    ///
    /// Returns [`DiscoveryError::NotComplete`] when an application response is still needed.
    pub fn finish(mut self) -> Result<ImageCatalog, DiscoveryError> {
        self.drive()?;
        self.catalog.take().ok_or(DiscoveryError::NotComplete)
    }

    fn drive(&mut self) -> Result<(), DiscoveryError> {
        while self.catalog.is_none() {
            let Some(index) = self.candidates.iter().position(|candidate| matches!(candidate.state, CandidateState::New) || matches!(candidate.state, CandidateState::Waiting(id) if self.outcomes.contains_key(&id))) else {
                if self.requests.values().any(|need| !self.outcomes.contains_key(&need.id)) { return Ok(()); }
                if self.candidates.iter().all(|candidate| matches!(candidate.state, CandidateState::Rejected)) {
                    return Err(DiscoveryError::NoCandidateAccepted { diagnostics: self.diagnostics.clone() });
                }
                return Ok(());
            };
            self.transitions += 1;
            if self.transitions > self.limits.transitions {
                return Err(DiscoveryError::TransitionLimitExceeded);
            }
            let result = self
                .advance_candidate(index)
                .and_then(|step| self.apply_step(index, step));
            match result {
                Ok(()) => {}
                Err(DiscoveryError::Session(message)) => {
                    self.reject_candidate(index, message);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn advance_candidate(&mut self, index: usize) -> Result<DiscoveryStep, DiscoveryError> {
        let candidate = &self.candidates[index];
        let state = match candidate.state {
            CandidateState::New => None,
            CandidateState::Waiting(id) => Some(id),
            CandidateState::Rejected => unreachable!("rejected candidates are not driven"),
        };

        if state.is_none() && !(candidate.spec.recognize)(&self.input.uri) {
            return Err(DiscoveryError::Session(candidate.spec.rejection.into()));
        }

        match (candidate.spec.program, state) {
            (DiscoveryProgram::Immediate(complete), None) => {
                complete(&self.input.uri).map(DiscoveryStep::Complete)
            }
            (DiscoveryProgram::Immediate(_), Some(_)) => {
                unreachable!("immediate dezoomers never follow resources")
            }
            (DiscoveryProgram::Rules { .. }, None) => {
                Ok(DiscoveryStep::Follow(Request::new(self.input.uri.clone())))
            }
            (DiscoveryProgram::Rules { routes, on_failure }, Some(id)) => {
                let request = &self.requests[&id].request;
                let previous_history = candidate
                    .history
                    .strip_suffix(&[id])
                    .expect("the current resource is last in candidate history");
                let context = DiscoveryContext {
                    history_ids: previous_history,
                    requests: &self.requests,
                    outcomes: &self.outcomes,
                };
                match self
                    .outcomes
                    .get(&id)
                    .expect("ready candidate has an outcome")
                {
                    ResourceOutcome::Response(response) => dispatch_resource(
                        routes,
                        &context,
                        DiscoveryResource { request, response },
                        "resource did not match any discovery route",
                    ),
                    ResourceOutcome::Failure(failure) => on_failure.map_or_else(
                        || Err(DiscoveryError::Session(failure.message.clone())),
                        |handler| handler(&context, DiscoveryFailure { request, failure }),
                    ),
                }
            }
        }
    }

    fn apply_step(&mut self, index: usize, step: DiscoveryStep) -> Result<(), DiscoveryError> {
        match step {
            DiscoveryStep::Follow(request) => {
                let request = match self.candidates[index].spec.program {
                    DiscoveryProgram::Rules { routes, .. } => map_url(routes, &request.uri)?,
                    DiscoveryProgram::Immediate(_) => request,
                };
                match self.register_request(request) {
                    Ok(id) => {
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
                    Err(DiscoveryError::ResourceLimitExceeded) => {
                        self.reject_candidate(index, "discovery resource limit exceeded".into());
                    }
                    Err(error) => return Err(error),
                }
            }
            DiscoveryStep::Complete(catalog) => {
                let catalog = catalog
                    .normalize()
                    .map_err(|error| DiscoveryError::Session(error.to_string()))?;
                self.catalog = Some(catalog);
            }
        }
        Ok(())
    }

    fn reject_candidate(&mut self, index: usize, diagnostic: String) {
        let id = self.candidates[index].spec.name.to_owned();
        self.diagnostics.push((id, diagnostic));
        self.candidates[index].state = CandidateState::Rejected;
    }

    fn register_request(&mut self, request: Request) -> Result<RequestId, DiscoveryError> {
        if let Some(id) = self.request_ids.get(&request).copied() {
            return Ok(id);
        }
        if self.requests.len() >= self.limits.resources {
            return Err(DiscoveryError::ResourceLimitExceeded);
        }
        let id = RequestId(self.next_request_id);
        self.next_request_id += 1;
        self.request_ids.insert(request.clone(), id);
        self.requests.insert(id, ResourceNeed { id, request });
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::registry::Registry;

    fn catalog(_: &str, _: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
        Ok(ImageCatalog::default())
    }

    fn reject(
        _: &DiscoveryContext<'_>,
        _: DiscoveryResource<'_>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        Err(DiscoveryError::Session("wrong format".into()))
    }

    const COMPLETE: &[DiscoveryRoute] = &[DiscoveryMatch::any().extract(catalog)];
    const REJECT: &[DiscoveryRoute] = &[DiscoveryMatch::any().then(reject)];

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

    fn is_tile(uri: &str) -> bool {
        uri.ends_with("/tile.jpg")
    }

    fn tile_metadata(uri: &str) -> Result<Request, DiscoveryError> {
        Ok(Request::new(uri.replace("/tile.jpg", "/metadata")))
    }

    const MAPPED: &[DiscoveryRoute] = &[
        DiscoveryMatch::url_matching(is_tile).map_url(tile_metadata),
        DiscoveryMatch::url_suffix("/metadata").extract(catalog),
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
        Ok(DiscoveryStep::follow(format!("{}/tiles", resource.uri())))
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
        DiscoveryMatch::url_suffix("/metadata").then(follow_tiles),
        DiscoveryMatch::url_suffix("/tiles").then(finish_chain),
    ];

    #[test]
    fn followed_resources_are_redispatched_with_history() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::new("chain", CHAIN));
        let mut operation = registry.start("memory://image/metadata");
        provide(&mut operation, b"metadata");
        assert_eq!(
            operation.next_priority_need().unwrap().unwrap().request.uri,
            "memory://image/metadata/tiles"
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
    }

    fn follow_a(
        context: &DiscoveryContext<'_>,
        _: DiscoveryResource<'_>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        assert!(context.resources().next().is_none());
        Ok(DiscoveryStep::follow("memory://a"))
    }

    fn follow_b(
        context: &DiscoveryContext<'_>,
        _: DiscoveryResource<'_>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        assert!(context.resources().next().is_none());
        Ok(DiscoveryStep::follow("memory://b"))
    }

    const HISTORY_A: &[DiscoveryRoute] = &[DiscoveryMatch::any().then(follow_a)];
    const HISTORY_B: &[DiscoveryRoute] = &[DiscoveryMatch::any().then(follow_b)];

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
        failure: DiscoveryFailure<'_>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        assert_eq!(failure.uri(), "memory://failure");
        assert_eq!(failure.failure().message, "expected");
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
        Ok(DiscoveryStep::follow(resource.uri()))
    }

    const REPEAT: &[DiscoveryRoute] = &[DiscoveryMatch::any().then(repeat)];

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
        Ok(DiscoveryStep::follow(format!(
            "memory://{}",
            context.resources().count()
        )))
    }

    const LOOP: &[DiscoveryRoute] = &[DiscoveryMatch::any().then(follow_again)];

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

        let mut resources = registry.start_with_limits(
            "memory://start",
            DiscoveryLimits {
                resources: 1,
                ..Default::default()
            },
        );
        let need = resources.missing_resources().unwrap().pop().unwrap();
        assert!(
            resources
                .provide(ResourceResponse::new(need.id, []))
                .is_err()
        );

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
        Ok(DiscoveryStep::follow("memory://high-2"))
    }

    const HIGH: &[DiscoveryRoute] = &[
        DiscoveryMatch::url_matching(is_root).map_url(high_start),
        DiscoveryMatch::url_suffix("high-1").then(high_next),
        DiscoveryMatch::url_suffix("high-2").extract(catalog),
    ];
    const LOW: &[DiscoveryRoute] = &[
        DiscoveryMatch::url_matching(is_root).map_url(low_start),
        DiscoveryMatch::any().extract(catalog),
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
    fn media_type_matching_ignores_case_and_parameters() {
        let request = Request::new("memory://page");
        let response = ResourceResponse::new(RequestId(0), b"page")
            .with_content_type("Text/HTML; charset=utf-8");
        let resource = DiscoveryResource {
            request: &request,
            response: &response,
        };
        assert!(DiscoveryMatch::media_type(&["text/html"]).matches_resource(resource));
        assert!(!DiscoveryMatch::media_type(&["text/xml"]).matches_resource(resource));
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
