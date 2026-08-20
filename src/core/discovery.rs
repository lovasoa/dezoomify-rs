//! Pure, resumable discovery orchestration.
//!
//! A discovery program only describes resource work.  The application owns
//! fetching and repeatedly feeds outcomes back to [`DiscoveryOperation`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use super::model::{ImageCatalog, Provenance, ProvenanceStep, Request, StableId};

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

/// Why a program requested a resource.  It is informational and never
/// prescribes transport.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ResourcePurpose {
    InitialMetadata,
    Metadata,
    ViewerScript,
    TileInformation,
}

/// A resource which must be supplied by the application before discovery can
/// continue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceNeed {
    pub id: RequestId,
    pub request: Request,
    pub purpose: ResourcePurpose,
}

/// A resource request before the operation assigns its stable ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceRequest {
    pub request: Request,
    pub purpose: ResourcePurpose,
}

impl ResourceRequest {
    #[must_use]
    pub fn new(uri: impl Into<String>, purpose: ResourcePurpose) -> Self {
        Self {
            request: Request::new(uri),
            purpose,
        }
    }
}

/// Bytes supplied by the application for a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceResponse {
    pub id: RequestId,
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
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
    #[cfg_attr(not(test), allow(dead_code))]
    Delegate(Delegation),
}

/// Start another registered program, optionally applying named profiles to
/// every boundary of the delegated discovery operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Delegation {
    pub program_id: String,
    pub input: DiscoveryInput,
    pub profiles: Vec<String>,
}

impl Delegation {
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn new(program_id: impl Into<String>, input: impl Into<DiscoveryInput>) -> Self {
        Self {
            program_id: program_id.into(),
            input: input.into(),
            profiles: Vec::new(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn with_profiles(mut self, profiles: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.profiles = profiles.into_iter().map(Into::into).collect();
        self
    }
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

/// An explicit, named adaptation selected by a delegation.
pub trait Profile: Send + Sync {
    /// Adapt the input before the delegated program starts.
    ///
    /// The default leaves the input unchanged.
    fn adapt_input(&self, input: DiscoveryInput) -> Result<DiscoveryInput, DiscoveryError> {
        Ok(input)
    }

    /// Adapt a resource request emitted by the delegated program.
    ///
    /// The default leaves the request unchanged.
    fn adapt_request(&self, request: ResourceRequest) -> Result<ResourceRequest, DiscoveryError> {
        Ok(request)
    }

    /// Adapt an application-supplied resource result before the delegated
    /// program receives it.
    ///
    /// The default leaves the outcome unchanged.
    fn adapt_resource(&self, outcome: ResourceOutcome) -> Result<ResourceOutcome, DiscoveryError> {
        Ok(outcome)
    }

    /// Adapt the final catalog emitted by the delegated program.
    ///
    /// # Errors
    ///
    /// Returns a pure diagnostic when the catalog cannot be adapted.
    fn adapt_catalog(&self, catalog: ImageCatalog) -> Result<ImageCatalog, DiscoveryError> {
        Ok(catalog)
    }
}

/// Pure discovery errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscoveryError {
    UnknownRequest(RequestId),
    RequestAlreadyProvided(RequestId),
    UnknownDelegatedProgram(String),
    UnknownProfile(String),
    DuplicateDelegation {
        program_id: String,
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
            Self::RequestAlreadyProvided(id) => write!(f, "request {} was already supplied", id.0),
            Self::UnknownDelegatedProgram(id) => write!(f, "unknown delegated program '{id}'"),
            Self::UnknownProfile(id) => write!(f, "unknown discovery profile '{id}'"),
            Self::DuplicateDelegation { program_id, uri } => write!(
                f,
                "program '{program_id}' was already delegated for '{uri}'"
            ),
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
    profiles: Vec<String>,
    provenance: Provenance,
    ancestry: BTreeSet<(String, String)>,
    session: Box<dyn DiscoverySession>,
    state: CandidateState,
}

/// A pull-driven operation with no I/O or shared mutable parser state.
pub struct DiscoveryOperation {
    programs: BTreeMap<String, Arc<dyn DiscoveryProgram>>,
    profiles: BTreeMap<String, Arc<dyn Profile>>,
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
        profiles: BTreeMap<String, Arc<dyn Profile>>,
        limits: DiscoveryLimits,
    ) -> Self {
        let mut by_id = BTreeMap::new();
        let mut candidates = Vec::new();
        for (id, program) in programs {
            by_id.insert(id.clone(), Arc::clone(&program));
            let ancestry = BTreeSet::from([(id.clone(), input.uri.clone())]);
            candidates.push(Candidate {
                id,
                profiles: Vec::new(),
                provenance: Provenance::default(),
                ancestry,
                session: program.start(input),
                state: CandidateState::New,
            });
        }
        // Registry validation and URL ranking supply the canonical candidate
        // order. Keep it intact here; sorting again would discard URL hints.
        Self {
            programs: by_id,
            profiles,
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
                let profiles = self.candidates[index].profiles.clone();
                let outcome = self.adapt_resource(outcome, &profiles)?;
                self.candidates[index]
                    .session
                    .advance(DiscoveryEvent::Resource(&outcome))
            }
        }
    }

    fn apply_step(&mut self, index: usize, step: DiscoveryStep) -> Result<(), DiscoveryError> {
        match step {
            DiscoveryStep::Need(request) => {
                let profiles = self.candidates[index].profiles.clone();
                self.candidates[index].state = CandidateState::Waiting(
                    self.register_request(self.adapt_request(request, &profiles)?)?,
                );
            }
            DiscoveryStep::Complete(catalog) => {
                let profiles = self.candidates[index].profiles.clone();
                let provenance = self.candidates[index].provenance.clone();
                self.catalog = Some(self.apply_profiles(catalog, &profiles, provenance)?);
            }
            DiscoveryStep::Reject(diagnostic) => {
                self.reject_candidate(index, diagnostic);
            }
            DiscoveryStep::Delegate(delegation) => self.delegate(index, delegation)?,
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
                purpose: request.purpose,
            },
        );
        Ok(id)
    }

    fn delegate(
        &mut self,
        candidate_index: usize,
        delegation: Delegation,
    ) -> Result<(), DiscoveryError> {
        let mut profiles = self.candidates[candidate_index].profiles.clone();
        profiles.extend(delegation.profiles.iter().cloned());
        let key = (delegation.program_id.clone(), delegation.input.uri.clone());
        let mut ancestry = self.candidates[candidate_index].ancestry.clone();
        if !ancestry.insert(key.clone()) {
            return Err(DiscoveryError::DuplicateDelegation {
                program_id: key.0,
                uri: key.1,
            });
        }
        let Some(program) = self.programs.get(&delegation.program_id).cloned() else {
            return Err(DiscoveryError::UnknownDelegatedProgram(
                delegation.program_id,
            ));
        };
        for profile in &delegation.profiles {
            if !self.profiles.contains_key(profile) {
                return Err(DiscoveryError::UnknownProfile(profile.clone()));
            }
        }
        let input = self.adapt_input(delegation.input, &profiles)?;
        let from = self.candidates[candidate_index].id.clone();
        let mut provenance = self.candidates[candidate_index].provenance.clone();
        provenance.0.push(ProvenanceStep {
            id: StableId::new(from.clone()),
            description: format!("delegated to {} for {}", delegation.program_id, input.uri),
        });
        self.candidates[candidate_index].state = CandidateState::Rejected;
        // Replace the rule in place so its explicit priority is retained and
        // lower-priority auto candidates cannot win before the delegated
        // format advances.
        self.candidates[candidate_index] = Candidate {
            id: delegation.program_id,
            profiles,
            provenance,
            ancestry,
            session: program.start(&input),
            state: CandidateState::New,
        };
        Ok(())
    }

    fn adapt_input(
        &self,
        input: DiscoveryInput,
        profile_ids: &[String],
    ) -> Result<DiscoveryInput, DiscoveryError> {
        self.adapt(input, profile_ids, |profile, input| {
            profile.adapt_input(input)
        })
    }

    fn adapt_request(
        &self,
        request: ResourceRequest,
        profile_ids: &[String],
    ) -> Result<ResourceRequest, DiscoveryError> {
        self.adapt(request, profile_ids, |profile, request| {
            profile.adapt_request(request)
        })
    }

    fn adapt_resource(
        &self,
        outcome: ResourceOutcome,
        profile_ids: &[String],
    ) -> Result<ResourceOutcome, DiscoveryError> {
        self.adapt(outcome, profile_ids, |profile, outcome| {
            profile.adapt_resource(outcome)
        })
    }

    fn adapt<T>(
        &self,
        value: T,
        profile_ids: &[String],
        adapt: impl Fn(&dyn Profile, T) -> Result<T, DiscoveryError>,
    ) -> Result<T, DiscoveryError> {
        profile_ids.iter().try_fold(value, |value, id| {
            let profile = self
                .profiles
                .get(id)
                .ok_or_else(|| DiscoveryError::UnknownProfile(id.clone()))?;
            adapt(profile.as_ref(), value)
        })
    }

    fn apply_profiles(
        &mut self,
        mut catalog: ImageCatalog,
        profile_ids: &[String],
        mut provenance: Provenance,
    ) -> Result<ImageCatalog, DiscoveryError> {
        for id in profile_ids {
            let Some(profile) = self.profiles.get(id) else {
                return Err(DiscoveryError::UnknownProfile(id.clone()));
            };
            catalog = profile.adapt_catalog(catalog)?;
            provenance.0.push(ProvenanceStep {
                id: StableId::new(id.clone()),
                description: "applied discovery profile".into(),
            });
        }
        let mut catalog = catalog
            .normalize()
            .map_err(|error| DiscoveryError::Session(error.to_string()))?;
        catalog.append_provenance(&provenance);
        Ok(catalog)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{CatalogEntry, DeferredImage, StableId};
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
        Delegate {
            program: &'static str,
            target: &'static str,
            profile: Option<&'static str>,
        },
        Target,
        Boundary,
    }

    struct ScriptProgram(Script);

    struct ScriptSession {
        script: Script,
        input: String,
        started: bool,
    }

    impl DiscoveryProgram for ScriptProgram {
        fn start(&self, input: &DiscoveryInput) -> Box<dyn DiscoverySession> {
            Box::new(ScriptSession {
                script: self.0.clone(),
                input: input.uri.clone(),
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
                    Ok(DiscoveryStep::Need(ResourceRequest::new(
                        *uri,
                        ResourcePurpose::Metadata,
                    )))
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
                    Script::Delegate {
                        program,
                        target,
                        profile,
                    },
                    DiscoveryEvent::Start,
                ) => {
                    let delegation = Delegation::new(*program, *target);
                    Ok(DiscoveryStep::Delegate(match profile {
                        Some(profile) => delegation.with_profiles([*profile]),
                        None => delegation,
                    }))
                }
                (Script::Target, DiscoveryEvent::Start) => Ok(DiscoveryStep::Complete(
                    deferred_catalog("target", "memory://target"),
                )),
                (Script::Boundary, DiscoveryEvent::Start)
                    if self.input != "memory://target#profiled" =>
                {
                    Ok(DiscoveryStep::Reject("unprofiled input".into()))
                }
                (Script::Boundary, DiscoveryEvent::Start) if !self.started => {
                    self.started = true;
                    Ok(DiscoveryStep::Need(ResourceRequest::new(
                        "memory://target-metadata",
                        ResourcePurpose::Metadata,
                    )))
                }
                (
                    Script::Boundary,
                    DiscoveryEvent::Resource(ResourceOutcome::Response(response)),
                ) if response.bytes == b"repaired" => Ok(DiscoveryStep::Complete(
                    deferred_catalog("boundary", self.input.clone()),
                )),
                (Script::Boundary, DiscoveryEvent::Resource(_)) => {
                    Ok(DiscoveryStep::Reject("unrepaired resource".into()))
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
                (Script::Need { .. }, _) | (Script::Delegate { .. }, _) | (Script::Target, _) => {
                    Err(DiscoveryError::Session("unexpected test transition".into()))
                }
                (Script::Boundary, _) => Err(DiscoveryError::Session(
                    "unexpected boundary transition".into(),
                )),
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
                content_type: None,
            })
            .unwrap();
    }

    fn need_program(uri: &'static str, result: NeedResult) -> Arc<ScriptProgram> {
        Arc::new(ScriptProgram(Script::Need { uri, result }))
    }

    fn deferred_catalog(id: &str, uri: impl Into<String>) -> ImageCatalog {
        ImageCatalog::new([CatalogEntry::Deferred(DeferredImage {
            id: StableId::from(id),
            uri: uri.into(),
            title: None,
            provenance: Provenance::default(),
            warnings: Vec::new(),
        })])
    }

    /*
     * The scripted programs below deliberately share one session harness: the
     * tests exercise operation boundaries, not independent parser plumbing.
     */
    struct WarningProfile {
        warning: &'static str,
    }

    impl Profile for WarningProfile {
        fn adapt_catalog(&self, mut catalog: ImageCatalog) -> Result<ImageCatalog, DiscoveryError> {
            for entry in &mut catalog.0 {
                if let CatalogEntry::Deferred(image) = entry {
                    image.warnings.push(self.warning.into());
                }
            }
            Ok(catalog)
        }
    }

    struct RepairingProfile {
        adapt_catalog: fn(ImageCatalog) -> Result<ImageCatalog, DiscoveryError>,
    }

    impl Profile for RepairingProfile {
        fn adapt_input(&self, mut input: DiscoveryInput) -> Result<DiscoveryInput, DiscoveryError> {
            input.uri.push_str("#profiled");
            Ok(input)
        }

        fn adapt_request(
            &self,
            mut request: ResourceRequest,
        ) -> Result<ResourceRequest, DiscoveryError> {
            request
                .request
                .headers
                .insert("X-Profile".into(), "boundary".into());
            Ok(request)
        }

        fn adapt_resource(
            &self,
            outcome: ResourceOutcome,
        ) -> Result<ResourceOutcome, DiscoveryError> {
            match outcome {
                ResourceOutcome::Response(mut response) => {
                    response.bytes = b"repaired".to_vec();
                    Ok(ResourceOutcome::Response(response))
                }
                failure @ ResourceOutcome::Failure(_) => Ok(failure),
            }
        }

        fn adapt_catalog(&self, catalog: ImageCatalog) -> Result<ImageCatalog, DiscoveryError> {
            (self.adapt_catalog)(catalog)
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    fn add_profile_warning(mut catalog: ImageCatalog) -> Result<ImageCatalog, DiscoveryError> {
        let CatalogEntry::Deferred(image) = &mut catalog.0[0] else {
            unreachable!("boundary test catalog is deferred")
        };
        image.warnings.push("profiled".into());
        Ok(catalog)
    }

    fn reject_catalog(_: ImageCatalog) -> Result<ImageCatalog, DiscoveryError> {
        Err(DiscoveryError::Session(
            "profile cannot repair catalog".into(),
        ))
    }

    #[allow(clippy::unnecessary_wraps)]
    fn duplicate_catalog(_: ImageCatalog) -> Result<ImageCatalog, DiscoveryError> {
        let entry = CatalogEntry::Deferred(DeferredImage {
            id: StableId::from("duplicate"),
            uri: "memory://duplicate".into(),
            title: None,
            provenance: Provenance::default(),
            warnings: Vec::new(),
        });
        Ok(ImageCatalog::new([entry.clone(), entry]))
    }

    fn repairing_operation(
        profile: &'static str,
        adapt_catalog: fn(ImageCatalog) -> Result<ImageCatalog, DiscoveryError>,
        fallback: bool,
    ) -> DiscoveryOperation {
        let mut registry = Registry::new();
        registry.register(
            "root",
            Priority(0),
            Arc::new(ScriptProgram(Script::Delegate {
                program: "boundary-target",
                target: "memory://target",
                profile: Some(profile),
            })),
        );
        registry.register(
            "boundary-target",
            Priority(1),
            Arc::new(ScriptProgram(Script::Boundary)),
        );
        if fallback {
            registry.register(
                "fallback",
                Priority(2),
                need_program("memory://fallback", NeedResult::Complete),
            );
        }
        registry.register_profile(profile, Arc::new(RepairingProfile { adapt_catalog }));
        registry.start("memory://root").unwrap()
    }

    #[test]
    fn identical_requests_are_fanned_out_to_candidates_once() {
        let mut registry = Registry::new();
        registry.register(
            "rejecting",
            Priority(0),
            need_program("rejecting", "memory://shared", NeedResult::Reject),
        );
        registry.register(
            "accepting",
            Priority(1),
            need_program("accepting", "memory://shared", NeedResult::Complete),
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
            need_program("broken", "memory://unused", NeedResult::Session),
        );
        registry.register(
            "working",
            Priority(1),
            need_program("working", "memory://working", NeedResult::Complete),
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
            need_program("one", "memory://metadata", NeedResult::Complete),
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

    #[test]
    fn profile_adapts_every_delegated_discovery_boundary() {
        let mut operation = repairing_operation("boundary", add_profile_warning, false);
        let needs = operation.missing_resources().unwrap();
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].request.uri, "memory://target-metadata");
        assert_eq!(
            needs[0].request.headers.get("X-Profile"),
            Some(&"boundary".into())
        );
        operation
            .provide(ResourceResponse {
                id: needs[0].id,
                bytes: b"broken".to_vec(),
                content_type: None,
            })
            .unwrap();

        let catalog = operation.finish().unwrap();
        let CatalogEntry::Deferred(image) = &catalog.0[0] else {
            panic!("expected deferred image")
        };
        assert_eq!(image.uri, "memory://target#profiled");
        assert_eq!(image.warnings, ["profiled"]);
        assert!(
            image
                .provenance
                .0
                .iter()
                .any(|step| step.id.as_str() == "boundary")
        );
    }

    #[test]
    fn profile_session_error_rejects_its_candidate_and_uses_fallback() {
        let mut operation = repairing_operation("failing-catalog", reject_catalog, true);
        let needs = operation.missing_resources().unwrap();
        assert_eq!(needs.len(), 2);
        let target_need = needs
            .iter()
            .find(|need| need.request.uri == "memory://target-metadata")
            .unwrap();
        operation
            .provide(ResourceResponse {
                id: target_need.id,
                bytes: b"broken".to_vec(),
                content_type: None,
            })
            .unwrap();
        assert!(operation.diagnostics.iter().any(|(id, diagnostic)| {
            id == "boundary-target" && diagnostic.message == "profile cannot repair catalog"
        }));
        let needs = operation.missing_resources().unwrap();
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].request.uri, "memory://fallback");
        operation
            .provide(ResourceResponse {
                id: needs[0].id,
                bytes: Vec::new(),
                content_type: None,
            })
            .unwrap();
        assert!(operation.finish().unwrap().is_empty());
    }

    #[test]
    fn catalog_normalization_error_rejects_its_candidate_and_uses_fallback() {
        let mut operation = repairing_operation("malformed-catalog", duplicate_catalog, true);
        let target_need = operation
            .missing_resources()
            .unwrap()
            .into_iter()
            .find(|need| need.request.uri == "memory://target-metadata")
            .unwrap();
        operation
            .provide(ResourceResponse {
                id: target_need.id,
                bytes: b"broken".to_vec(),
                content_type: None,
            })
            .unwrap();
        assert!(operation.diagnostics.iter().any(|(id, diagnostic)| {
            id == "boundary-target" && diagnostic.message.contains("duplicate")
        }));
        let fallback_need = operation.missing_resources().unwrap().pop().unwrap();
        assert_eq!(fallback_need.request.uri, "memory://fallback");
        operation
            .provide(ResourceResponse {
                id: fallback_need.id,
                bytes: Vec::new(),
                content_type: None,
            })
            .unwrap();
        assert!(operation.finish().unwrap().is_empty());
    }

    #[test]
    fn delegation_inherits_explicit_profiles_and_records_provenance() {
        let mut registry = Registry::new();
        registry.register(
            "root",
            Priority(0),
            Arc::new(DelegateProgram {
                target_program: "middle",
                target: "memory://middle",
                profile: Some("outer"),
            }),
        );
        registry.register(
            "middle",
            Priority(1),
            Arc::new(DelegateProgram {
                target_program: "target",
                target: "memory://target",
                profile: Some("inner"),
            }),
        );
        registry.register("target", Priority(2), Arc::new(TargetProgram));
        registry.register_profile("outer", Arc::new(WarningProfile { warning: "outer" }));
        registry.register_profile("inner", Arc::new(WarningProfile { warning: "inner" }));

        let mut operation = registry.start("memory://root").unwrap();
        assert!(operation.missing_resources().unwrap().is_empty());
        let catalog = operation.finish().unwrap();
        let CatalogEntry::Deferred(image) = &catalog.0[0] else {
            panic!("expected deferred test image");
        };
        assert_eq!(image.warnings, ["outer", "inner"]);
        assert_eq!(
            image
                .provenance
                .0
                .iter()
                .map(|step| step.id.as_str())
                .collect::<Vec<_>>(),
            ["root", "middle", "outer", "inner"]
        );
    }
}
