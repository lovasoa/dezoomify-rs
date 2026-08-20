//! Pure, resumable discovery orchestration.
//!
//! This module intentionally contains no transport or runtime integration.  A
//! caller pulls [`ResourceNeed`] values, obtains their bytes however it likes,
//! and supplies the result back to a [`DiscoveryOperation`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use super::model::ImageCatalog;

/// An input to a discovery operation.  A URI is deliberately opaque: it may
/// identify a network resource, a local resource, or an application-defined
/// source.
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

/// A stable identifier for one resource within one discovery operation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RequestId(pub u64);

/// Why a format asked for a resource.  This is descriptive only; it never
/// instructs the application to use a particular transport.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ResourcePurpose {
    InitialMetadata,
    Metadata,
    DiscoveryRule,
    Profile,
    ViewerScript,
    TileInformation,
    Other(String),
}

/// Portable request requirements.  Ordered collections keep generated plans
/// and request identities deterministic.
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct RequestRequirements {
    pub headers: BTreeMap<String, String>,
}

/// A not-yet-satisfied resource request emitted by discovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceNeed {
    pub id: RequestId,
    pub uri: String,
    pub purpose: ResourcePurpose,
    pub accepted_content_types: BTreeSet<String>,
    pub requirements: RequestRequirements,
}

/// A request before an operation has assigned its stable [`RequestId`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceRequest {
    pub uri: String,
    pub purpose: ResourcePurpose,
    pub accepted_content_types: BTreeSet<String>,
    pub requirements: RequestRequirements,
}

impl ResourceRequest {
    #[must_use]
    pub fn new(uri: impl Into<String>, purpose: ResourcePurpose) -> Self {
        Self {
            uri: uri.into(),
            purpose,
            accepted_content_types: BTreeSet::new(),
            requirements: RequestRequirements::default(),
        }
    }
}

/// Bytes supplied by the application for a requested resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceResponse {
    pub id: RequestId,
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

/// An application-reported failure to satisfy a requested resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceFailure {
    pub id: RequestId,
    pub message: String,
}

/// Explicit bounds applied to one discovery operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryLimits {
    pub max_transitions: usize,
    pub max_resources: usize,
    pub max_retained_bytes: usize,
}

impl Default for DiscoveryLimits {
    fn default() -> Self {
        Self {
            max_transitions: 10_000,
            max_resources: 256,
            max_retained_bytes: 64 * 1024 * 1024,
        }
    }
}

/// The supplied result visible to a waiting parser.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceOutcome {
    Response(ResourceResponse),
    Failure(ResourceFailure),
}

/// A deterministic diagnostic retained when a candidate declines an input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryDiagnostic {
    pub message: String,
}

impl From<String> for DiscoveryDiagnostic {
    fn from(message: String) -> Self {
        Self { message }
    }
}

impl From<&str> for DiscoveryDiagnostic {
    fn from(message: &str) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// A format or discovery-rule transition.
pub enum SessionStep {
    Need(ResourceRequest),
    Complete(ImageCatalog),
    Reject(DiscoveryDiagnostic),
    Delegate(Delegation),
}

/// Delegates a discovered candidate to an already-registered base format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Delegation {
    pub format_id: String,
    pub input: DiscoveryInput,
}

/// Object-safe, pure protocol implementation.
pub trait FormatHandler: Send + Sync {
    fn id(&self) -> &'static str;
    fn start(&self, input: &DiscoveryInput) -> Box<dyn FormatSession>;
}

/// Parser state owned by exactly one discovery operation.
pub trait FormatSession: Send {
    /// Advance the parser from its initial input.
    ///
    /// # Errors
    ///
    /// Returns an error when supplied input cannot be interpreted by this
    /// session.
    fn start(&mut self, input: &DiscoveryInput) -> Result<SessionStep, DiscoveryError>;

    /// Advance the parser with a resource outcome previously requested by it.
    ///
    /// # Errors
    ///
    /// Returns an error when the outcome cannot be interpreted by this
    /// session.
    fn provide(&mut self, resource: &ResourceOutcome) -> Result<SessionStep, DiscoveryError>;
}

/// A composable pure discovery rule.  Rules can request metadata and delegate
/// the extracted candidate to a registered format without copying its parser.
pub trait DiscoveryRule: Send + Sync {
    fn id(&self) -> &'static str;
    fn start(&self, input: &DiscoveryInput) -> Box<dyn DiscoveryRuleSession>;
}

/// Rule state owned by exactly one discovery operation.
pub trait DiscoveryRuleSession: Send {
    /// Advance the rule from its initial input.
    ///
    /// # Errors
    ///
    /// Returns an error when supplied input cannot be interpreted by this
    /// rule.
    fn start(&mut self, input: &DiscoveryInput) -> Result<SessionStep, DiscoveryError>;

    /// Advance the rule with a resource outcome previously requested by it.
    ///
    /// # Errors
    ///
    /// Returns an error when the outcome cannot be interpreted by this rule.
    fn provide(&mut self, resource: &ResourceOutcome) -> Result<SessionStep, DiscoveryError>;
}

/// A narrowly-scoped adaptation applied after a base format has produced a
/// catalog.  The operation records every applied profile in provenance.
pub trait Profile: Send + Sync {
    fn id(&self) -> &'static str;

    /// Adapt a catalog produced by a base format.
    ///
    /// # Errors
    ///
    /// Returns an error when the catalog cannot be adapted by this profile.
    fn apply(&self, catalog: ImageCatalog) -> Result<ImageCatalog, DiscoveryError>;
}

/// Deterministic provenance emitted by orchestration.  Format-specific
/// provenance belongs in the catalog values produced by handlers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProvenanceEvent {
    ResourceRequested {
        id: RequestId,
        uri: String,
    },
    ResourceProvided {
        id: RequestId,
    },
    ResourceFailed {
        id: RequestId,
        message: String,
    },
    RuleDelegated {
        rule_id: String,
        format_id: String,
        uri: String,
    },
    ProfileApplied {
        profile_id: String,
    },
    CandidateRejected {
        candidate_id: String,
        message: String,
    },
}

/// Discovery errors are pure values; applications decide how to present them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscoveryError {
    UnknownRequest(RequestId),
    RequestAlreadyProvided(RequestId),
    UnknownDelegatedFormat(String),
    DuplicateDelegation {
        format_id: String,
        uri: String,
    },
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
            Self::RequestAlreadyProvided(id) => {
                write!(f, "discovery request {} was already provided", id.0)
            }
            Self::UnknownDelegatedFormat(id) => write!(f, "unknown delegated format '{id}'"),
            Self::DuplicateDelegation { format_id, uri } => {
                write!(f, "format '{format_id}' was already delegated for '{uri}'")
            }
            Self::NoCandidateAccepted { diagnostics } => {
                write!(f, "no discovery candidate accepted the input")?;
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
                f.write_str("discovery retained-metadata size limit exceeded")
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

#[derive(Clone)]
pub(crate) struct RegisteredFormat {
    pub(crate) id: String,
    pub(crate) priority: i32,
    pub(crate) handler: Arc<dyn FormatHandler>,
}

#[derive(Clone)]
pub(crate) struct RegisteredRule {
    pub(crate) id: String,
    pub(crate) priority: i32,
    pub(crate) rule: Arc<dyn DiscoveryRule>,
}

#[derive(Clone)]
pub(crate) struct RegisteredProfile {
    pub(crate) id: String,
    pub(crate) profile: Arc<dyn Profile>,
}

enum CandidateKind {
    Format(Box<dyn FormatSession>),
    Rule(Box<dyn DiscoveryRuleSession>),
}

enum CandidateState {
    New,
    Waiting(RequestId),
    Rejected,
}

struct Candidate {
    id: String,
    priority: i32,
    input: DiscoveryInput,
    kind: CandidateKind,
    state: CandidateState,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RequestKey {
    uri: String,
    requirements: RequestRequirements,
}

/// A pull-driven, operation-local discovery state machine.
pub struct DiscoveryOperation {
    formats: BTreeMap<String, Arc<dyn FormatHandler>>,
    profiles: Vec<RegisteredProfile>,
    candidates: Vec<Candidate>,
    requests: BTreeMap<RequestId, ResourceNeed>,
    request_ids: BTreeMap<RequestKey, RequestId>,
    outcomes: BTreeMap<RequestId, ResourceOutcome>,
    delegated: BTreeSet<(String, String)>,
    diagnostics: Vec<(String, DiscoveryDiagnostic)>,
    provenance: Vec<ProvenanceEvent>,
    catalog: Option<ImageCatalog>,
    next_request_id: u64,
    transitions: usize,
    retained_bytes: usize,
    limits: DiscoveryLimits,
}

impl DiscoveryOperation {
    pub(crate) fn new(
        input: &DiscoveryInput,
        formats: Vec<RegisteredFormat>,
        rules: Vec<RegisteredRule>,
        profiles: Vec<RegisteredProfile>,
        limits: DiscoveryLimits,
    ) -> Self {
        let mut handlers = BTreeMap::new();
        let mut candidates = Vec::new();
        for format in formats {
            handlers.insert(format.id.clone(), Arc::clone(&format.handler));
            candidates.push(Candidate {
                id: format.id,
                priority: format.priority,
                input: input.clone(),
                kind: CandidateKind::Format(format.handler.start(input)),
                state: CandidateState::New,
            });
        }
        for rule in rules {
            candidates.push(Candidate {
                id: rule.id,
                priority: rule.priority,
                input: input.clone(),
                kind: CandidateKind::Rule(rule.rule.start(input)),
                state: CandidateState::New,
            });
        }
        candidates.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.id.cmp(&right.id))
        });
        Self {
            formats: handlers,
            profiles,
            candidates,
            requests: BTreeMap::new(),
            request_ids: BTreeMap::new(),
            outcomes: BTreeMap::new(),
            delegated: BTreeSet::new(),
            diagnostics: Vec::new(),
            provenance: Vec::new(),
            catalog: None,
            next_request_id: 0,
            transitions: 0,
            retained_bytes: 0,
            limits,
        }
    }

    /// Return every still-unsatisfied request in stable request-ID order.
    ///
    /// # Errors
    ///
    /// Returns an error when a candidate cannot advance or no candidate can
    /// accept the supplied operation state.
    pub fn missing_resources(&mut self) -> Result<Vec<ResourceNeed>, DiscoveryError> {
        self.drive()?;
        Ok(self
            .requests
            .iter()
            .filter(|(id, _)| !self.outcomes.contains_key(id))
            .map(|(_, need)| need.clone())
            .collect())
    }

    /// Supply successful resource bytes.  Results are retained only by this
    /// operation and fan out to all candidates waiting for the request.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown or already-supplied request, or when a
    /// candidate cannot advance from the response.
    pub fn provide(&mut self, response: ResourceResponse) -> Result<(), DiscoveryError> {
        let id = response.id;
        self.provide_outcome(id, ResourceOutcome::Response(response))
    }

    /// Supply a failed resource result without imposing retry policy on core.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown or already-supplied request, or when a
    /// candidate cannot advance from the failure.
    pub fn provide_failure(&mut self, failure: ResourceFailure) -> Result<(), DiscoveryError> {
        let id = failure.id;
        self.provide_outcome(id, ResourceOutcome::Failure(failure))
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
                .filter(|total| *total <= self.limits.max_retained_bytes)
                .ok_or(DiscoveryError::MetadataSizeLimitExceeded)?;
        }
        match &outcome {
            ResourceOutcome::Response(_) => self
                .provenance
                .push(ProvenanceEvent::ResourceProvided { id }),
            ResourceOutcome::Failure(failure) => {
                self.provenance.push(ProvenanceEvent::ResourceFailed {
                    id,
                    message: failure.message.clone(),
                });
            }
        }
        self.outcomes.insert(id, outcome);
        self.drive()
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.catalog.is_some()
    }

    #[must_use]
    pub fn provenance(&self) -> &[ProvenanceEvent] {
        &self.provenance
    }

    /// Consume the completed operation and return its immutable catalog.
    ///
    /// # Errors
    ///
    /// Returns an error when discovery cannot advance or is still waiting for
    /// resources.
    pub fn finish(mut self) -> Result<ImageCatalog, DiscoveryError> {
        self.drive()?;
        self.catalog.take().ok_or(DiscoveryError::NotComplete)
    }

    fn drive(&mut self) -> Result<(), DiscoveryError> {
        while self.catalog.is_none() {
            self.transitions += 1;
            if self.transitions > self.limits.max_transitions {
                return Err(DiscoveryError::TransitionLimitExceeded);
            }

            let next = self
                .candidates
                .iter()
                .position(|candidate| match candidate.state {
                    CandidateState::New => true,
                    CandidateState::Waiting(id) => self.outcomes.contains_key(&id),
                    CandidateState::Rejected => false,
                });
            let Some(index) = next else {
                if self
                    .requests
                    .values()
                    .any(|need| !self.outcomes.contains_key(&need.id))
                {
                    return Ok(());
                }
                if self
                    .candidates
                    .iter()
                    .all(|candidate| matches!(candidate.state, CandidateState::Rejected))
                {
                    return Err(DiscoveryError::NoCandidateAccepted {
                        diagnostics: self.diagnostics.clone(),
                    });
                }
                return Ok(());
            };

            let step =
                {
                    let candidate = &mut self.candidates[index];
                    match (&mut candidate.kind, &candidate.state) {
                        (CandidateKind::Format(session), CandidateState::New) => {
                            session.start(&candidate.input)
                        }
                        (CandidateKind::Rule(session), CandidateState::New) => {
                            session.start(&candidate.input)
                        }
                        (CandidateKind::Format(session), CandidateState::Waiting(id)) => session
                            .provide(self.outcomes.get(id).expect("ready candidate has outcome")),
                        (CandidateKind::Rule(session), CandidateState::Waiting(id)) => session
                            .provide(self.outcomes.get(id).expect("ready candidate has outcome")),
                        (_, CandidateState::Rejected) => continue,
                    }?
                };
            self.apply_step(index, step)?;
        }
        Ok(())
    }

    fn apply_step(&mut self, index: usize, step: SessionStep) -> Result<(), DiscoveryError> {
        match step {
            SessionStep::Need(request) => {
                let id = self.register_request(request)?;
                self.candidates[index].state = CandidateState::Waiting(id);
            }
            SessionStep::Complete(catalog) => {
                self.catalog = Some(self.apply_profiles(catalog)?);
            }
            SessionStep::Reject(diagnostic) => {
                let id = self.candidates[index].id.clone();
                self.provenance.push(ProvenanceEvent::CandidateRejected {
                    candidate_id: id.clone(),
                    message: diagnostic.message.clone(),
                });
                self.diagnostics.push((id, diagnostic));
                self.candidates[index].state = CandidateState::Rejected;
            }
            SessionStep::Delegate(delegation) => self.delegate(index, delegation)?,
        }
        Ok(())
    }

    fn register_request(&mut self, request: ResourceRequest) -> Result<RequestId, DiscoveryError> {
        let key = RequestKey {
            uri: request.uri.clone(),
            requirements: request.requirements.clone(),
        };
        if let Some(id) = self.request_ids.get(&key).copied() {
            let need = self
                .requests
                .get_mut(&id)
                .expect("registered request is retained");
            need.accepted_content_types
                .extend(request.accepted_content_types);
            return Ok(id);
        }
        if self.requests.len() >= self.limits.max_resources {
            return Err(DiscoveryError::ResourceLimitExceeded);
        }
        let id = RequestId(self.next_request_id);
        self.next_request_id += 1;
        self.request_ids.insert(key, id);
        self.requests.insert(
            id,
            ResourceNeed {
                id,
                uri: request.uri.clone(),
                purpose: request.purpose,
                accepted_content_types: request.accepted_content_types,
                requirements: request.requirements,
            },
        );
        self.provenance.push(ProvenanceEvent::ResourceRequested {
            id,
            uri: request.uri,
        });
        Ok(id)
    }

    fn delegate(
        &mut self,
        candidate_index: usize,
        delegation: Delegation,
    ) -> Result<(), DiscoveryError> {
        let key = (delegation.format_id.clone(), delegation.input.uri.clone());
        if !self.delegated.insert(key.clone()) {
            return Err(DiscoveryError::DuplicateDelegation {
                format_id: key.0,
                uri: key.1,
            });
        }
        let Some(handler) = self.formats.get(&delegation.format_id).cloned() else {
            return Err(DiscoveryError::UnknownDelegatedFormat(delegation.format_id));
        };
        let rule_id = self.candidates[candidate_index].id.clone();
        self.provenance.push(ProvenanceEvent::RuleDelegated {
            rule_id,
            format_id: delegation.format_id.clone(),
            uri: delegation.input.uri.clone(),
        });
        let priority = self.candidates[candidate_index].priority;
        self.candidates[candidate_index].state = CandidateState::Rejected;
        self.candidates.push(Candidate {
            id: delegation.format_id,
            priority,
            input: delegation.input.clone(),
            kind: CandidateKind::Format(handler.start(&delegation.input)),
            state: CandidateState::New,
        });
        Ok(())
    }

    fn apply_profiles(
        &mut self,
        mut catalog: ImageCatalog,
    ) -> Result<ImageCatalog, DiscoveryError> {
        for profile in &self.profiles {
            catalog = profile.profile.apply(catalog)?;
            self.provenance.push(ProvenanceEvent::ProfileApplied {
                profile_id: profile.id.clone(),
            });
        }
        Ok(catalog)
    }
}
