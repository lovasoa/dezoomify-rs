//! Pure, resumable discovery orchestration.
//!
//! A discovery program only describes resource work.  The application owns
//! fetching and repeatedly feeds outcomes back to [`DiscoveryOperation`].

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

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

/// Factory for independent, pure parser sessions.  Formats and URL discovery
/// rules implement this same trait; priority is registry policy.
pub trait DiscoveryProgram: Send + Sync {
    fn start(&self, input: &DiscoveryInput) -> Box<dyn DiscoverySession>;
}

/// One operation-local parser state machine.
pub trait DiscoverySession: Send {
    /// Advance the parser from an input or one requested resource result.
    ///
    /// # Errors
    ///
    /// Returns a pure diagnostic when bytes or state cannot be interpreted.
    fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError>;
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
    session: Box<dyn DiscoverySession>,
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
        programs: Vec<(String, Arc<dyn DiscoveryProgram>)>,
        limits: DiscoveryLimits,
    ) -> Self {
        let mut candidates = Vec::new();
        for (id, program) in programs {
            candidates.push(Candidate {
                id,
                session: program.start(input),
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
        Ok(self
            .requests
            .iter()
            .filter(|(id, _)| !self.outcomes.contains_key(id))
            .map(|(_, need)| need.clone())
            .collect())
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
            DiscoveryStep::Need(request) => {
                self.candidates[index].state = CandidateState::Waiting(
                    self.register_request(request)?,
                );
            }
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
    use crate::core::registry::{Priority, Registry};

    #[derive(Clone, Copy)]
    enum NeedResult {
        Complete,
        Reject,
        Session,
    }

    #[derive(Clone)]
    enum Script {
        Need {
            uri: &'static str,
            result: NeedResult,
        },
    }

    struct ScriptProgram(Script);

    struct ScriptSession {
        script: Script,
        started: bool,
    }

    impl DiscoveryProgram for ScriptProgram {
        fn start(&self, input: &DiscoveryInput) -> Box<dyn DiscoverySession> {
            let _ = input;
            Box::new(ScriptSession {
                script: self.0.clone(),
                started: false,
            })
        }
    }

    impl DiscoverySession for ScriptSession {
        fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
            match (&self.script, event) {
                (
                    Script::Need {
                        result: NeedResult::Session,
                        ..
                    },
                    DiscoveryEvent::Start,
                ) => Err(DiscoveryError::Session("not my format".into())),
                (Script::Need { uri, .. }, DiscoveryEvent::Start) if !self.started => {
                    self.started = true;
                    Ok(DiscoveryStep::Need(ResourceRequest::new(*uri)))
                }
                (
                    Script::Need {
                        result: NeedResult::Complete,
                        ..
                    },
                    DiscoveryEvent::Resource(ResourceOutcome::Response(_)),
                ) => Ok(DiscoveryStep::Complete(ImageCatalog::default())),
                (
                    Script::Need {
                        result: NeedResult::Reject,
                        ..
                    },
                    DiscoveryEvent::Resource(ResourceOutcome::Response(_)),
                ) => Ok(DiscoveryStep::Reject("wrong metadata".into())),
                (Script::Need { .. }, DiscoveryEvent::Resource(ResourceOutcome::Failure(_))) => {
                    Ok(DiscoveryStep::Reject("resource unavailable".into()))
                }
                (
                    Script::Need {
                        result: NeedResult::Session,
                        ..
                    },
                    _,
                ) => {
                    unreachable!("failed session never requests a resource")
                }
                (Script::Need { .. }, _) => {
                    Err(DiscoveryError::Session("unexpected test transition".into()))
                }
            }
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

    fn need_program(uri: &'static str, result: NeedResult) -> Arc<ScriptProgram> {
        Arc::new(ScriptProgram(Script::Need { uri, result }))
    }

    #[test]
    fn identical_requests_are_fanned_out_to_candidates_once() {
        let mut registry = Registry::new();
        registry.register(
            "rejecting",
            Priority(0),
            need_program("memory://shared", NeedResult::Reject),
        );
        registry.register(
            "accepting",
            Priority(1),
            need_program("memory://shared", NeedResult::Complete),
        );

        let mut operation = registry.start("memory://root").unwrap();
        assert_eq!(operation.missing_resources().unwrap().len(), 1);
        need_response(&mut operation, b"metadata");
        assert!(operation.is_complete());
    }

    #[test]
    fn session_errors_reject_one_candidate_and_continue() {
        let mut registry = Registry::new();
        registry.register(
            "broken",
            Priority(0),
            need_program("memory://unused", NeedResult::Session),
        );
        registry.register(
            "working",
            Priority(1),
            need_program("memory://working", NeedResult::Complete),
        );

        let mut operation = registry.start("memory://root").unwrap();
        let needs = operation.missing_resources().unwrap();
        assert_eq!(needs[0].request.uri, "memory://working");
        provide_response(&mut operation, needs[0].id, b"");
        assert!(operation.finish().unwrap().is_empty());
    }

    #[test]
    fn parser_state_is_local_to_each_operation() {
        let mut registry = Registry::new();
        registry.register(
            "one",
            Priority(0),
            need_program("memory://metadata", NeedResult::Complete),
        );
        let mut first = registry.start("memory://first").unwrap();
        let mut second = registry.start("memory://second").unwrap();
        let first_id = first.missing_resources().unwrap()[0].id;
        let second_id = second.missing_resources().unwrap()[0].id;
        assert_eq!(first_id, second_id);
        provide_response(&mut first, first_id, b"");
        assert!(first.is_complete());
        assert!(!second.is_complete());
        assert_eq!(second.missing_resources().unwrap().len(), 1);
    }
}
