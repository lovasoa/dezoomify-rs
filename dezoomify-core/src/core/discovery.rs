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
/// This replaces the former `DiscoveryProgram`/`DiscoverySession` pair. A
/// dezoomer is boxed as `dyn Dezoomer`, so it stays object-safe; static
/// metadata and construction live on [`DezoomerMeta`].
pub trait Dezoomer: Send {
    /// Advance the parser from an input or one requested resource result.
    ///
    /// # Errors
    ///
    /// Returns a pure diagnostic when bytes or state cannot be interpreted.
    fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError>;
}

/// Static metadata and construction for a concrete [`Dezoomer`] type.
///
/// [`Dezoomer`] cannot carry associated consts without losing object safety,
/// so the name, URL hints, and constructor live here instead. A format
/// implements both traits on one struct.
pub trait DezoomerMeta: Dezoomer + Sized {
    /// Public name: the `--dezoomer` selector and the diagnostic label.
    const NAME: &'static str;

    /// Cheap URL fragments used only to rank auto-detection candidates.
    ///
    /// The authoritative accept/reject decision stays in [`Dezoomer::advance`].
    const URL_HINTS: &'static [&'static str] = &[];

    /// Build fresh, independent parser state from the input.
    fn start(input: &DiscoveryInput) -> Self;
}

/// Erased constructor for a concrete [`DezoomerMeta`] type.
#[must_use]
pub fn erase<T: DezoomerMeta + 'static>(input: &DiscoveryInput) -> Box<dyn Dezoomer> {
    Box::new(T::start(input))
}

/// The single declaration point for a format: its name, URL hints, and
/// constructor, all derived from a concrete [`DezoomerMeta`] type.
#[derive(Clone, Copy)]
pub struct DezoomerSpec {
    pub name: &'static str,
    pub url_hints: &'static [&'static str],
    pub start: fn(&DiscoveryInput) -> Box<dyn Dezoomer>,
}

impl DezoomerSpec {
    /// Build a spec from a concrete dezoomer type.
    #[must_use]
    pub const fn of<T: DezoomerMeta + 'static>() -> Self {
        Self {
            name: T::NAME,
            url_hints: T::URL_HINTS,
            start: erase::<T>,
        }
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

enum CandidateState {
    New,
    Waiting(RequestId),
    Rejected,
}

struct Candidate {
    id: String,
    session: Box<dyn Dezoomer>,
    state: CandidateState,
}

/// A pull-driven operation with no I/O or shared mutable parser state.
pub struct DiscoveryOperation {
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
                id: spec.name.to_owned(),
                session: (spec.start)(input),
                state: CandidateState::New,
            });
        }
        // Registry validation and URL ranking supply the canonical candidate
        // order. Keep it intact here; sorting again would discard URL hints.
        Self {
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
        let state = match self.candidates[index].state {
            CandidateState::New => None,
            CandidateState::Waiting(id) => Some(id),
            CandidateState::Rejected => unreachable!("rejected candidates are not driven"),
        };
        match state {
            None => self.candidates[index]
                .session
                .advance(DiscoveryEvent::Start),
            Some(id) => {
                let outcome = self
                    .outcomes
                    .get(&id)
                    .expect("ready candidate has an outcome")
                    .clone();
                self.candidates[index]
                    .session
                    .advance(DiscoveryEvent::Resource(&outcome))
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
        let id = self.candidates[index].id.clone();
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

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum NeedResult {
        Complete,
        Reject,
        Session,
    }

    /// Generates a single-request dezoomer that requests `$uri` on start and
    /// then completes, rejects, or errors according to `$result`.
    macro_rules! script_dezoomer {
        ($ty:ident, $name:literal, $uri:literal, $result:path) => {
            struct $ty {
                started: bool,
            }

            impl Dezoomer for $ty {
                fn advance(
                    &mut self,
                    event: DiscoveryEvent<'_>,
                ) -> Result<DiscoveryStep, DiscoveryError> {
                    match event {
                        DiscoveryEvent::Start if $result == NeedResult::Session => {
                            Err(DiscoveryError::Session("not my format".into()))
                        }
                        DiscoveryEvent::Start if !self.started => {
                            self.started = true;
                            Ok(DiscoveryStep::Need(ResourceRequest::new($uri)))
                        }
                        DiscoveryEvent::Resource(ResourceOutcome::Response(_))
                            if $result == NeedResult::Complete =>
                        {
                            Ok(DiscoveryStep::Complete(ImageCatalog::default()))
                        }
                        DiscoveryEvent::Resource(ResourceOutcome::Response(_))
                            if $result == NeedResult::Reject =>
                        {
                            Ok(DiscoveryStep::Reject("wrong metadata".into()))
                        }
                        DiscoveryEvent::Resource(ResourceOutcome::Failure(_)) => {
                            Ok(DiscoveryStep::Reject("resource unavailable".into()))
                        }
                        DiscoveryEvent::Resource(ResourceOutcome::Response(_)) => {
                            unreachable!("failed session never requests a resource")
                        }
                        DiscoveryEvent::Start => {
                            Err(DiscoveryError::Session("unexpected test transition".into()))
                        }
                    }
                }
            }

            impl DezoomerMeta for $ty {
                const NAME: &'static str = $name;

                fn start(_: &DiscoveryInput) -> Self {
                    Self { started: false }
                }
            }
        };
    }

    /// Generates a two-stage dezoomer that requests `$first`, then `$second`,
    /// then completes.
    macro_rules! chain_dezoomer {
        ($ty:ident, $name:literal, $first:literal, $second:literal) => {
            struct $ty {
                state: u8,
            }

            impl Dezoomer for $ty {
                fn advance(
                    &mut self,
                    event: DiscoveryEvent<'_>,
                ) -> Result<DiscoveryStep, DiscoveryError> {
                    match (self.state, event) {
                        (0, DiscoveryEvent::Start) => {
                            self.state = 1;
                            Ok(DiscoveryStep::Need(ResourceRequest::new($first)))
                        }
                        (1, DiscoveryEvent::Resource(ResourceOutcome::Response(_))) => {
                            self.state = 2;
                            Ok(DiscoveryStep::Need(ResourceRequest::new($second)))
                        }
                        (2, DiscoveryEvent::Resource(ResourceOutcome::Response(_))) => {
                            Ok(DiscoveryStep::Complete(ImageCatalog::default()))
                        }
                        _ => Ok(DiscoveryStep::Reject("unexpected".into())),
                    }
                }
            }

            impl DezoomerMeta for $ty {
                const NAME: &'static str = $name;

                fn start(_: &DiscoveryInput) -> Self {
                    Self { state: 0 }
                }
            }
        };
    }

    /// Generates a single-stage dezoomer that requests `$uri`, then completes.
    macro_rules! single_dezoomer {
        ($ty:ident, $name:literal, $uri:literal) => {
            struct $ty {
                requested: bool,
            }

            impl Dezoomer for $ty {
                fn advance(
                    &mut self,
                    event: DiscoveryEvent<'_>,
                ) -> Result<DiscoveryStep, DiscoveryError> {
                    match event {
                        DiscoveryEvent::Start if !self.requested => {
                            self.requested = true;
                            Ok(DiscoveryStep::Need(ResourceRequest::new($uri)))
                        }
                        DiscoveryEvent::Resource(ResourceOutcome::Response(_)) => {
                            Ok(DiscoveryStep::Complete(ImageCatalog::default()))
                        }
                        _ => Ok(DiscoveryStep::Reject("unexpected".into())),
                    }
                }
            }

            impl DezoomerMeta for $ty {
                const NAME: &'static str = $name;

                fn start(_: &DiscoveryInput) -> Self {
                    Self { requested: false }
                }
            }
        };
    }

    script_dezoomer!(
        RejectingShared,
        "rejecting",
        "memory://shared",
        NeedResult::Reject
    );
    script_dezoomer!(
        AcceptingShared,
        "accepting",
        "memory://shared",
        NeedResult::Complete
    );
    script_dezoomer!(Broken, "broken", "memory://unused", NeedResult::Session);
    script_dezoomer!(Working, "working", "memory://working", NeedResult::Complete);
    script_dezoomer!(
        OneMetadata,
        "one",
        "memory://metadata",
        NeedResult::Complete
    );
    script_dezoomer!(OneMeta, "one", "memory://meta", NeedResult::Complete);

    chain_dezoomer!(HighChain, "high", "memory://high1", "memory://high2");

    single_dezoomer!(Low, "low", "memory://low1");
    single_dezoomer!(A, "a", "memory://a");
    single_dezoomer!(B, "b", "memory://b");
    single_dezoomer!(C, "c", "memory://c");

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

    impl DezoomerMeta for Loop {
        const NAME: &'static str = "loop";

        fn start(_: &DiscoveryInput) -> Self {
            Self(0)
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

    #[test]
    fn identical_requests_are_fanned_out_to_candidates_once() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::of::<RejectingShared>());
        registry.register(DezoomerSpec::of::<AcceptingShared>());

        let mut operation = registry.start("memory://root");
        assert_eq!(operation.missing_resources().unwrap().len(), 1);
        need_response(&mut operation, b"metadata");
        assert!(operation.is_complete());
    }

    #[test]
    fn session_errors_reject_one_candidate_and_continue() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::of::<Broken>());
        registry.register(DezoomerSpec::of::<Working>());

        let mut operation = registry.start("memory://root");
        let needs = operation.missing_resources().unwrap();
        assert_eq!(needs[0].request.uri, "memory://working");
        provide_response(&mut operation, needs[0].id, b"");
        assert!(operation.finish().unwrap().is_empty());
    }

    #[test]
    fn parser_state_is_local_to_each_operation() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::of::<OneMetadata>());
        let mut first = registry.start("memory://first");
        let mut second = registry.start("memory://second");
        let first_id = first.missing_resources().unwrap()[0].id;
        let second_id = second.missing_resources().unwrap()[0].id;
        assert_eq!(first_id, second_id);
        provide_response(&mut first, first_id, b"");
        assert!(first.is_complete());
        assert!(!second.is_complete());
        assert_eq!(second.missing_resources().unwrap().len(), 1);
    }

    #[test]
    fn priority_scheduling_fetches_higher_priority_chain_first() {
        let mut registry = Registry::new();
        registry.register(DezoomerSpec::of::<HighChain>());
        registry.register(DezoomerSpec::of::<Low>());

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
        registry.register(DezoomerSpec::of::<Loop>());
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
        registry.register(DezoomerSpec::of::<A>());
        registry.register(DezoomerSpec::of::<B>());
        registry.register(DezoomerSpec::of::<C>());
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
        registry.register(DezoomerSpec::of::<OneMeta>());
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
