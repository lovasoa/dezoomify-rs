//! Pure, resumable discovery orchestration.
//!
//! A discovery program only describes resource work.  The application owns
//! fetching and repeatedly feeds outcomes back to [`DiscoveryOperation`].

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

/// A resource request before the operation assigns its stable ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceRequest {
    pub request: Request,
}

impl ResourceRequest {
    #[must_use]
    pub fn new(uri: impl Into<String>) -> Self {
        Self {
            request: Request::new(uri),
        }
    }
}

/// Bytes supplied by the application for a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceResponse {
    pub id: RequestId,
    pub bytes: Vec<u8>,
}

/// An application-reported failure to satisfy a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceFailure {
    pub id: RequestId,
    pub message: String,
}

/// The resource result visible to a waiting program.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceOutcome {
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

/// A deterministic reason a program did not recognize its input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryDiagnostic {
    pub message: String,
}

impl From<&str> for DiscoveryDiagnostic {
    fn from(message: &str) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<String> for DiscoveryDiagnostic {
    fn from(message: String) -> Self {
        Self { message }
    }
}

/// Input delivered to a program session.
pub enum DiscoveryEvent<'a> {
    /// The session's first transition.
    Start,
    /// A result for its preceding [`DiscoveryStep::Need`].
    Resource(&'a ResourceOutcome),
}

/// One pure transition yielded by a discovery session.
pub enum DiscoveryStep {
    Need(ResourceRequest),
    Complete(ImageCatalog),
    Reject(DiscoveryDiagnostic),
}

/// One dezoomer: a pure parser state machine driven by [`DiscoveryOperation`].
///
/// Complex formats remain free to implement an object-safe, multi-resource
/// state machine. Most formats use a routed [`DezoomerSpec`] instead.
pub trait Dezoomer: Send {
    /// Advance the parser from an input or one requested resource result.
    ///
    /// # Errors
    ///
    /// Returns a pure diagnostic when bytes or state cannot be interpreted.
    fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError>;
}

#[derive(Clone, Copy, Debug)]
enum SpecAdapter {
    Immediate(fn(&str) -> Result<ImageCatalog, DiscoveryError>),
    Resource {
        request: fn(&str) -> Result<ResourceRequest, DiscoveryError>,
        parse: fn(&str, &[u8]) -> Result<ImageCatalog, DiscoveryError>,
    },
    Stateful(fn(&DiscoveryInput) -> Box<dyn Dezoomer>),
}

/// One co-located format declaration.
#[derive(Clone, Copy, Debug)]
pub struct DezoomerSpec {
    name: &'static str,
    recognize: fn(&str) -> bool,
    rejection: &'static str,
    prefer: fn(&str) -> bool,
    adapter: SpecAdapter,
}

impl DezoomerSpec {
    /// Declare a format which completes from its input without a resource.
    #[must_use]
    pub const fn immediate(
        name: &'static str,
        complete: fn(&str) -> Result<ImageCatalog, DiscoveryError>,
    ) -> Self {
        Self::new(name, SpecAdapter::Immediate(complete))
    }

    /// Declare a format which parses one derived resource.
    #[must_use]
    pub const fn routed(
        name: &'static str,
        request: fn(&str) -> Result<ResourceRequest, DiscoveryError>,
        parse: fn(&str, &[u8]) -> Result<ImageCatalog, DiscoveryError>,
    ) -> Self {
        Self::new(name, SpecAdapter::Resource { request, parse })
    }

    /// Declare a format backed by an object-safe, multi-resource state machine.
    #[must_use]
    pub const fn stateful(
        name: &'static str,
        start: fn(&DiscoveryInput) -> Box<dyn Dezoomer>,
    ) -> Self {
        Self::new(name, SpecAdapter::Stateful(start))
    }

    const fn new(name: &'static str, adapter: SpecAdapter) -> Self {
        Self {
            name,
            recognize: |_| true,
            rejection: "input not recognized",
            prefer: |_| false,
            adapter,
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

/// Request the input URI unchanged.
pub fn input_resource(uri: &str) -> Result<ResourceRequest, DiscoveryError> {
    Ok(ResourceRequest::new(uri))
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
    NoCandidateAccepted {
        diagnostics: Vec<(String, DiscoveryDiagnostic)>,
    },
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
                    write!(f, "; {id}: {}", diagnostic.message)?;
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
    session: Option<Box<dyn Dezoomer>>,
    state: CandidateState,
}

/// A pull-driven operation with no I/O or shared mutable parser state.
pub struct DiscoveryOperation {
    input: DiscoveryInput,
    candidates: Vec<Candidate>,
    requests: BTreeMap<RequestId, ResourceNeed>,
    request_ids: BTreeMap<Request, RequestId>,
    outcomes: BTreeMap<RequestId, ResourceOutcome>,
    diagnostics: Vec<(String, DiscoveryDiagnostic)>,
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
                session: match spec.adapter {
                    SpecAdapter::Stateful(start) => Some(start(input)),
                    SpecAdapter::Immediate(_) | SpecAdapter::Resource { .. } => None,
                },
                state: CandidateState::New,
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
                    self.reject_candidate(index, DiscoveryDiagnostic::from(message));
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
            return Ok(DiscoveryStep::Reject(candidate.spec.rejection.into()));
        }

        match candidate.spec.adapter {
            SpecAdapter::Immediate(complete) => {
                debug_assert!(state.is_none());
                complete(&self.input.uri).map(DiscoveryStep::Complete)
            }
            SpecAdapter::Resource { request, parse } => match state {
                None => request(&self.input.uri).map(DiscoveryStep::Need),
                Some(id) => match self
                    .outcomes
                    .get(&id)
                    .expect("ready candidate has an outcome")
                {
                    ResourceOutcome::Response(response) => {
                        let uri = &self.requests[&id].request.uri;
                        parse(uri, &response.bytes).map(DiscoveryStep::Complete)
                    }
                    ResourceOutcome::Failure(failure) => {
                        Err(DiscoveryError::Session(failure.message.clone()))
                    }
                },
            },
            SpecAdapter::Stateful(_) => {
                let outcome = state.map(|id| {
                    self.outcomes
                        .get(&id)
                        .expect("ready candidate has an outcome")
                        .clone()
                });
                let session = self.candidates[index]
                    .session
                    .as_mut()
                    .expect("stateful adapter has a session");
                match outcome.as_ref() {
                    None => session.advance(DiscoveryEvent::Start),
                    Some(outcome) => session.advance(DiscoveryEvent::Resource(outcome)),
                }
            }
        }
    }

    fn apply_step(&mut self, index: usize, step: DiscoveryStep) -> Result<(), DiscoveryError> {
        match step {
            DiscoveryStep::Need(request) => match self.register_request(request) {
                Ok(id) => self.candidates[index].state = CandidateState::Waiting(id),
                Err(DiscoveryError::ResourceLimitExceeded) => {
                    self.reject_candidate(
                        index,
                        DiscoveryDiagnostic::from("discovery resource limit exceeded"),
                    );
                }
                Err(error) => return Err(error),
            },
            DiscoveryStep::Complete(catalog) => {
                let catalog = catalog
                    .normalize()
                    .map_err(|error| DiscoveryError::Session(error.to_string()))?;
                self.catalog = Some(catalog);
            }
            DiscoveryStep::Reject(diagnostic) => {
                self.reject_candidate(index, diagnostic);
            }
        }
        Ok(())
    }

    fn reject_candidate(&mut self, index: usize, diagnostic: DiscoveryDiagnostic) {
        let id = self.candidates[index].spec.name.to_owned();
        self.diagnostics.push((id, diagnostic));
        self.candidates[index].state = CandidateState::Rejected;
    }

    fn register_request(&mut self, request: ResourceRequest) -> Result<RequestId, DiscoveryError> {
        if let Some(id) = self.request_ids.get(&request.request).copied() {
            return Ok(id);
        }
        if self.requests.len() >= self.limits.resources {
            return Err(DiscoveryError::ResourceLimitExceeded);
        }
        let id = RequestId(self.next_request_id);
        self.next_request_id += 1;
        self.request_ids.insert(request.request.clone(), id);
        self.requests.insert(
            id,
            ResourceNeed {
                id,
                request: request.request,
            },
        );
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::registry::Registry;
    use crate::core::{CatalogEntry, DeferredImage, StableId};

    const REJECTING_SHARED: DezoomerSpec = DezoomerSpec::routed(
        "rejecting",
        |_| Ok(ResourceRequest::new("memory://shared")),
        |_, _| Err(DiscoveryError::Session("wrong metadata".into())),
    );
    const ACCEPTING_SHARED: DezoomerSpec = DezoomerSpec::routed(
        "accepting",
        |_| Ok(ResourceRequest::new("memory://shared")),
        |_, _| Ok(ImageCatalog::default()),
    );
    const BROKEN: DezoomerSpec = DezoomerSpec::immediate("broken", |_| {
        Err(DiscoveryError::Session("not my format".into()))
    });
    const WORKING: DezoomerSpec = DezoomerSpec::routed(
        "working",
        |_| Ok(ResourceRequest::new("memory://working")),
        |_, _| Ok(ImageCatalog::default()),
    );
    const ONE_META: DezoomerSpec = DezoomerSpec::routed(
        "one",
        |_| Ok(ResourceRequest::new("memory://meta")),
        |_, _| Ok(ImageCatalog::default()),
    );
    const LOW: DezoomerSpec = DezoomerSpec::routed(
        "low",
        |_| Ok(ResourceRequest::new("memory://low1")),
        |_, _| Ok(ImageCatalog::default()),
    );
    const A: DezoomerSpec = DezoomerSpec::routed(
        "a",
        |_| Ok(ResourceRequest::new("memory://a")),
        |_, _| Ok(ImageCatalog::default()),
    );
    const B: DezoomerSpec = DezoomerSpec::routed(
        "b",
        |_| Ok(ResourceRequest::new("memory://b")),
        |_, _| Ok(ImageCatalog::default()),
    );
    const C: DezoomerSpec = DezoomerSpec::routed(
        "c",
        |_| Ok(ResourceRequest::new("memory://c")),
        |_, _| Ok(ImageCatalog::default()),
    );

    struct Chain {
        state: u8,
        first: &'static str,
        second: &'static str,
    }

    impl Dezoomer for Chain {
        fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
            match (self.state, event) {
                (0, DiscoveryEvent::Start) => {
                    self.state = 1;
                    Ok(DiscoveryStep::Need(ResourceRequest::new(self.first)))
                }
                (1, DiscoveryEvent::Resource(ResourceOutcome::Response(_))) => {
                    self.state = 2;
                    Ok(DiscoveryStep::Need(ResourceRequest::new(self.second)))
                }
                (2, DiscoveryEvent::Resource(ResourceOutcome::Response(_))) => {
                    Ok(DiscoveryStep::Complete(ImageCatalog::default()))
                }
                _ => Ok(DiscoveryStep::Reject("unexpected".into())),
            }
        }
    }

    fn chain(first: &'static str, second: &'static str) -> Box<dyn Dezoomer> {
        Box::new(Chain {
            state: 0,
            first,
            second,
        })
    }

    const STATEFUL_CHAIN: DezoomerSpec =
        DezoomerSpec::stateful("chain", |_| chain("memory://metadata", "memory://tiles"));
    const HIGH_CHAIN: DezoomerSpec =
        DezoomerSpec::stateful("high", |_| chain("memory://high1", "memory://high2"));

    struct Loop(usize);
    impl Dezoomer for Loop {
        fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
            match event {
                DiscoveryEvent::Start => Ok(DiscoveryStep::Need(ResourceRequest::new(format!(
                    "memory://loop{}",
                    self.0
                )))),
                DiscoveryEvent::Resource(ResourceOutcome::Response(_)) => {
                    self.0 += 1;
                    Ok(DiscoveryStep::Need(ResourceRequest::new(format!(
                        "memory://loop{}",
                        self.0
                    ))))
                }
                DiscoveryEvent::Resource(ResourceOutcome::Failure(_)) => {
                    Ok(DiscoveryStep::Reject("done".into()))
                }
            }
        }
    }

    impl Loop {
        const SPEC: DezoomerSpec = DezoomerSpec::stateful("loop", Self::start);

        fn start(_: &DiscoveryInput) -> Box<dyn Dezoomer> {
            Box::new(Self(0))
        }
    }

    fn need_response(operation: &mut DiscoveryOperation, bytes: &[u8]) {
        let need = operation.missing_resources().unwrap();
        assert_eq!(need.len(), 1);
        provide_response(operation, need[0].id, bytes);
    }

    fn provide_response(operation: &mut DiscoveryOperation, id: RequestId, bytes: &[u8]) {
        operation
            .provide(ResourceResponse {
                id,
                bytes: bytes.to_vec(),
            })
            .unwrap();
    }

    fn derived_request(input: &str) -> ResourceRequest {
        ResourceRequest::new(format!("{input}/metadata"))
    }

    fn catalog_with_context(uri: &str) -> ImageCatalog {
        ImageCatalog::new([CatalogEntry::Deferred(DeferredImage {
            id: StableId::new("routed:0"),
            uri: uri.to_owned(),
            title: None,
            warnings: Vec::new(),
        })])
    }

    const ROUTED: DezoomerSpec = DezoomerSpec::routed(
        "routed",
        |uri| Ok(derived_request(uri)),
        |uri, _| Ok(catalog_with_context(uri)),
    )
    .recognizing(|uri| uri.starts_with("route:"), "not a routed input")
    .preferring(|uri| uri.ends_with("preferred"));

    #[test]
    fn route_adapter_recognizes_derives_and_parses_with_resource_context() {
        assert!(ROUTED.prefers("route:preferred"));
        let mut registry = Registry::new();
        registry.register(ROUTED);
        let mut operation = registry.start("route:input");
        let need = operation.missing_resources().unwrap().pop().unwrap();
        assert_eq!(need.request.uri, "route:input/metadata");
        provide_response(&mut operation, need.id, b"metadata");
        let catalog = operation.finish().unwrap();
        let [CatalogEntry::Deferred(image)] = catalog.entries() else {
            panic!("route parser should return its deferred image")
        };
        assert_eq!(image.uri, need.request.uri);

        let mut rejected = registry.start("other:input");
        assert!(matches!(
            rejected.missing_resources(),
            Err(DiscoveryError::NoCandidateAccepted { .. })
        ));
    }

    #[test]
    fn identical_requests_are_fanned_out_to_candidates_once() {
        let mut registry = Registry::new();
        registry.register(REJECTING_SHARED);
        registry.register(ACCEPTING_SHARED);

        let mut operation = registry.start("memory://root");
        assert_eq!(operation.missing_resources().unwrap().len(), 1);
        need_response(&mut operation, b"metadata");
        assert!(operation.is_complete());
    }

    #[test]
    fn session_errors_reject_one_candidate_and_continue() {
        let mut registry = Registry::new();
        registry.register(BROKEN);
        registry.register(WORKING);

        let mut operation = registry.start("memory://root");
        let needs = operation.missing_resources().unwrap();
        assert_eq!(needs[0].request.uri, "memory://working");
        provide_response(&mut operation, needs[0].id, b"");
        assert!(operation.finish().unwrap().is_empty());
    }

    #[test]
    fn parser_state_is_local_to_each_operation() {
        let mut registry = Registry::new();
        registry.register(STATEFUL_CHAIN);
        let mut first = registry.start("memory://first");
        let mut second = registry.start("memory://second");
        let first_id = first.missing_resources().unwrap()[0].id;
        let second_id = second.missing_resources().unwrap()[0].id;
        assert_eq!(first_id, second_id);
        provide_response(&mut first, first_id, b"");
        assert_eq!(
            first.next_priority_need().unwrap().unwrap().request.uri,
            "memory://tiles"
        );
        assert!(!second.is_complete());
        assert_eq!(
            second.next_priority_need().unwrap().unwrap().request.uri,
            "memory://metadata"
        );
    }

    #[test]
    fn priority_scheduling_fetches_higher_priority_chain_first() {
        let mut registry = Registry::new();
        registry.register(HIGH_CHAIN);
        registry.register(LOW);

        let mut operation = registry.start("memory://root");
        let needs = operation.missing_resources().unwrap();
        assert_eq!(needs.len(), 2);
        assert!(needs.iter().any(|n| n.request.uri == "memory://high1"));
        assert!(needs.iter().any(|n| n.request.uri == "memory://low1"));
        let next = operation.next_priority_need().unwrap().unwrap();
        assert_eq!(next.request.uri, "memory://high1");
        provide_response(&mut operation, next.id, b"");
        let needs = operation.missing_resources().unwrap();
        assert_eq!(needs.len(), 2);
        let prio = operation.next_priority_need().unwrap().unwrap();
        assert_eq!(
            prio.request.uri, "memory://high2",
            "higher-priority second request must be fetched before lower-priority first"
        );
        provide_response(&mut operation, prio.id, b"");
        assert!(operation.is_complete());
    }

    #[test]
    fn transition_limit_is_enforced() {
        let mut registry = Registry::new();
        registry.register(Loop::SPEC);
        let limits = DiscoveryLimits {
            transitions: 3,
            ..Default::default()
        };
        let mut operation = registry.start_with_limits("memory://root", limits);
        let mut result = Ok(());
        for _ in 0..5 {
            let needs = operation.missing_resources().unwrap();
            if needs.is_empty() {
                break;
            }
            result = operation.provide(ResourceResponse {
                id: needs[0].id,
                bytes: vec![],
            });
            if result.is_err() {
                break;
            }
        }
        assert!(matches!(
            result,
            Err(DiscoveryError::TransitionLimitExceeded)
        ));
    }

    #[test]
    fn resource_limit_rejects_offending_candidate_instead_of_aborting() {
        let mut registry = Registry::new();
        registry.register(A);
        registry.register(B);
        registry.register(C);
        let limits = DiscoveryLimits {
            resources: 2,
            ..Default::default()
        };
        let mut operation = registry.start_with_limits("memory://root", limits);
        let needs = operation.missing_resources().unwrap();
        assert_eq!(needs.len(), 2);
        assert!(
            operation
                .diagnostics
                .iter()
                .any(|(id, msg)| id == "c" && msg.message.contains("resource limit"))
        );
        for need in needs {
            operation
                .provide(ResourceResponse {
                    id: need.id,
                    bytes: vec![],
                })
                .unwrap();
        }
        assert!(operation.is_complete());
    }

    #[test]
    fn retained_bytes_limit_is_enforced() {
        let mut registry = Registry::new();
        registry.register(ONE_META);
        let limits = DiscoveryLimits {
            retained_bytes: 10,
            ..Default::default()
        };
        let mut operation = registry.start_with_limits("memory://root", limits);
        let needs = operation.missing_resources().unwrap();
        assert_eq!(needs.len(), 1);
        let large = vec![0u8; 20];
        let err = operation
            .provide(ResourceResponse {
                id: needs[0].id,
                bytes: large,
            })
            .unwrap_err();
        assert_eq!(err, DiscoveryError::MetadataSizeLimitExceeded);
    }
}
