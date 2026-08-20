//! Stable registration for pure formats, discovery rules, and profiles.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::discovery::{
    DiscoveryError, DiscoveryInput, DiscoveryLimits, DiscoveryOperation, DiscoveryRule,
    FormatHandler, Profile, RegisteredFormat, RegisteredProfile, RegisteredRule,
};

/// A stable registration identifier.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RegistrationId(pub String);

impl From<&str> for RegistrationId {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl From<String> for RegistrationId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// An explicit ordering key.  Lower numbers run first.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Priority(pub i32);

#[derive(Clone)]
struct FormatRegistration {
    id: RegistrationId,
    priority: Priority,
    handler: Arc<dyn FormatHandler>,
}

#[derive(Clone)]
struct RuleRegistration {
    id: RegistrationId,
    priority: Priority,
    rule: Arc<dyn DiscoveryRule>,
}

#[derive(Clone)]
struct ProfileRegistration {
    id: RegistrationId,
    priority: Priority,
    profile: Arc<dyn Profile>,
}

/// Registration failures are deterministic and independent of source order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryError {
    DuplicateId {
        kind: &'static str,
        id: String,
    },
    AmbiguousPriority {
        kind: &'static str,
        priority: Priority,
        ids: Vec<String>,
    },
    HandlerIdMismatch {
        registration_id: String,
        handler_id: String,
    },
    RuleIdMismatch {
        registration_id: String,
        rule_id: String,
    },
    ProfileIdMismatch {
        registration_id: String,
        profile_id: String,
    },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateId { kind, id } => write!(f, "duplicate {kind} registration ID '{id}'"),
            Self::AmbiguousPriority {
                kind,
                priority,
                ids,
            } => write!(
                f,
                "ambiguous {kind} precedence at priority {}: {}",
                priority.0,
                ids.join(", ")
            ),
            Self::HandlerIdMismatch {
                registration_id,
                handler_id,
            } => write!(
                f,
                "format registration ID '{registration_id}' does not match handler ID '{handler_id}'"
            ),
            Self::RuleIdMismatch {
                registration_id,
                rule_id,
            } => write!(
                f,
                "rule registration ID '{registration_id}' does not match rule ID '{rule_id}'"
            ),
            Self::ProfileIdMismatch {
                registration_id,
                profile_id,
            } => write!(
                f,
                "profile registration ID '{registration_id}' does not match profile ID '{profile_id}'"
            ),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Pure registration data.  Applications may retain a registry globally, but
/// every call to [`Registry::start`] creates independent parser state.
#[derive(Default, Clone)]
pub struct Registry {
    formats: Vec<FormatRegistration>,
    rules: Vec<RuleRegistration>,
    profiles: Vec<ProfileRegistration>,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_format(
        &mut self,
        id: impl Into<RegistrationId>,
        priority: Priority,
        handler: Arc<dyn FormatHandler>,
    ) {
        self.formats.push(FormatRegistration {
            id: id.into(),
            priority,
            handler,
        });
    }

    pub fn register_rule(
        &mut self,
        id: impl Into<RegistrationId>,
        priority: Priority,
        rule: Arc<dyn DiscoveryRule>,
    ) {
        self.rules.push(RuleRegistration {
            id: id.into(),
            priority,
            rule,
        });
    }

    pub fn register_profile(
        &mut self,
        id: impl Into<RegistrationId>,
        priority: Priority,
        profile: Arc<dyn Profile>,
    ) {
        self.profiles.push(ProfileRegistration {
            id: id.into(),
            priority,
            profile,
        });
    }

    /// Validate stable IDs and explicit precedence before any discovery starts.
    ///
    /// # Errors
    ///
    /// Returns an error for a duplicate registration ID, a registration whose
    /// implementation reports a different ID, or ambiguous precedence.
    pub fn validate(&self) -> Result<(), RegistryError> {
        validate_formats(&self.formats)?;
        validate_rules(&self.rules)?;
        validate_profiles(&self.profiles)?;
        validate_priorities(
            "discovery candidate",
            self.formats
                .iter()
                .map(|format| (format.priority, format.id.0.clone()))
                .chain(
                    self.rules
                        .iter()
                        .map(|rule| (rule.priority, rule.id.0.clone())),
                ),
        )?;
        Ok(())
    }

    /// Start a fully independent pure discovery operation.
    ///
    /// # Errors
    ///
    /// Returns a validation error when registrations have duplicate IDs,
    /// mismatched IDs, or ambiguous precedence.
    pub fn start(
        &self,
        input: impl Into<DiscoveryInput>,
    ) -> Result<DiscoveryOperation, RegistryError> {
        self.start_with_limits(input, DiscoveryLimits::default())
    }

    /// Start an independent operation with explicit resource and metadata bounds.
    ///
    /// # Errors
    ///
    /// Returns a registration error when IDs or priorities are invalid.
    pub fn start_with_limits(
        &self,
        input: impl Into<DiscoveryInput>,
        limits: DiscoveryLimits,
    ) -> Result<DiscoveryOperation, RegistryError> {
        self.validate()?;
        let mut formats = self.formats.clone();
        formats.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.id.cmp(&b.id)));
        let mut rules = self.rules.clone();
        rules.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.id.cmp(&b.id)));
        let mut profiles = self.profiles.clone();
        profiles.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.id.cmp(&b.id)));
        let input = input.into();
        Ok(DiscoveryOperation::new(
            &input,
            formats
                .into_iter()
                .map(|format| RegisteredFormat {
                    id: format.id.0,
                    priority: format.priority.0,
                    handler: format.handler,
                })
                .collect(),
            rules
                .into_iter()
                .map(|rule| RegisteredRule {
                    id: rule.id.0,
                    priority: rule.priority.0,
                    rule: rule.rule,
                })
                .collect(),
            profiles
                .into_iter()
                .map(|profile| RegisteredProfile {
                    id: profile.id.0,
                    profile: profile.profile,
                })
                .collect(),
            limits,
        ))
    }
}

fn validate_formats(registrations: &[FormatRegistration]) -> Result<(), RegistryError> {
    let mut ids = BTreeSet::new();
    for registration in registrations {
        if !ids.insert(registration.id.0.clone()) {
            return Err(RegistryError::DuplicateId {
                kind: "format",
                id: registration.id.0.clone(),
            });
        }
        if registration.handler.id() != registration.id.0 {
            return Err(RegistryError::HandlerIdMismatch {
                registration_id: registration.id.0.clone(),
                handler_id: registration.handler.id().into(),
            });
        }
    }
    validate_priorities(
        "format",
        registrations.iter().map(|r| (r.priority, r.id.0.clone())),
    )
}

fn validate_rules(registrations: &[RuleRegistration]) -> Result<(), RegistryError> {
    let mut ids = BTreeSet::new();
    for registration in registrations {
        if !ids.insert(registration.id.0.clone()) {
            return Err(RegistryError::DuplicateId {
                kind: "rule",
                id: registration.id.0.clone(),
            });
        }
        if registration.rule.id() != registration.id.0 {
            return Err(RegistryError::RuleIdMismatch {
                registration_id: registration.id.0.clone(),
                rule_id: registration.rule.id().into(),
            });
        }
    }
    validate_priorities(
        "rule",
        registrations.iter().map(|r| (r.priority, r.id.0.clone())),
    )
}

fn validate_profiles(registrations: &[ProfileRegistration]) -> Result<(), RegistryError> {
    let mut ids = BTreeSet::new();
    for registration in registrations {
        if !ids.insert(registration.id.0.clone()) {
            return Err(RegistryError::DuplicateId {
                kind: "profile",
                id: registration.id.0.clone(),
            });
        }
        if registration.profile.id() != registration.id.0 {
            return Err(RegistryError::ProfileIdMismatch {
                registration_id: registration.id.0.clone(),
                profile_id: registration.profile.id().into(),
            });
        }
    }
    validate_priorities(
        "profile",
        registrations.iter().map(|r| (r.priority, r.id.0.clone())),
    )
}

fn validate_priorities(
    kind: &'static str,
    entries: impl Iterator<Item = (Priority, String)>,
) -> Result<(), RegistryError> {
    let mut by_priority: BTreeMap<Priority, Vec<String>> = BTreeMap::new();
    for (priority, id) in entries {
        by_priority.entry(priority).or_default().push(id);
    }
    if let Some((priority, mut ids)) = by_priority.into_iter().find(|(_, ids)| ids.len() > 1) {
        ids.sort();
        return Err(RegistryError::AmbiguousPriority {
            kind,
            priority,
            ids,
        });
    }
    Ok(())
}

impl From<RegistryError> for DiscoveryError {
    fn from(error: RegistryError) -> Self {
        Self::Session(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::discovery::{
        Delegation, DiscoveryDiagnostic, DiscoveryRuleSession, FormatSession, ProvenanceEvent,
        ResourceOutcome, ResourcePurpose, ResourceRequest, SessionStep,
    };
    use crate::core::model::ImageCatalog;

    struct NeedMetadata;

    impl FormatHandler for NeedMetadata {
        fn id(&self) -> &'static str {
            "need-metadata"
        }

        fn start(&self, _input: &DiscoveryInput) -> Box<dyn FormatSession> {
            Box::new(NeedMetadataSession)
        }
    }

    struct NeedMetadataSession;

    impl FormatSession for NeedMetadataSession {
        fn start(&mut self, _input: &DiscoveryInput) -> Result<SessionStep, DiscoveryError> {
            Ok(SessionStep::Need(ResourceRequest::new(
                "memory://metadata",
                ResourcePurpose::Metadata,
            )))
        }

        fn provide(&mut self, resource: &ResourceOutcome) -> Result<SessionStep, DiscoveryError> {
            match resource {
                ResourceOutcome::Response(response) if response.bytes == b"metadata" => {
                    Ok(SessionStep::Complete(ImageCatalog::default()))
                }
                _ => Ok(SessionStep::Reject("unexpected test resource".into())),
            }
        }
    }

    struct RejectAtRoot;

    impl FormatHandler for RejectAtRoot {
        fn id(&self) -> &'static str {
            "base"
        }

        fn start(&self, input: &DiscoveryInput) -> Box<dyn FormatSession> {
            Box::new(BaseSession {
                input: input.clone(),
            })
        }
    }

    struct BaseSession {
        input: DiscoveryInput,
    }

    impl FormatSession for BaseSession {
        fn start(&mut self, _input: &DiscoveryInput) -> Result<SessionStep, DiscoveryError> {
            if self.input.uri == "memory://delegated" {
                Ok(SessionStep::Complete(ImageCatalog::default()))
            } else {
                Ok(SessionStep::Reject("only accepts delegated input".into()))
            }
        }

        fn provide(&mut self, _resource: &ResourceOutcome) -> Result<SessionStep, DiscoveryError> {
            Err(DiscoveryError::Session(
                "base session needs no resources".into(),
            ))
        }
    }

    struct DelegateRule;

    impl DiscoveryRule for DelegateRule {
        fn id(&self) -> &'static str {
            "synthetic-rule"
        }

        fn start(&self, _input: &DiscoveryInput) -> Box<dyn DiscoveryRuleSession> {
            Box::new(DelegateRuleSession)
        }
    }

    struct DelegateRuleSession;

    impl DiscoveryRuleSession for DelegateRuleSession {
        fn start(&mut self, _input: &DiscoveryInput) -> Result<SessionStep, DiscoveryError> {
            Ok(SessionStep::Delegate(Delegation {
                format_id: "base".into(),
                input: DiscoveryInput::from("memory://delegated"),
            }))
        }

        fn provide(&mut self, _resource: &ResourceOutcome) -> Result<SessionStep, DiscoveryError> {
            Err(DiscoveryError::Session("rule needs no resources".into()))
        }
    }

    struct RecordingProfile;

    impl Profile for RecordingProfile {
        fn id(&self) -> &'static str {
            "synthetic-profile"
        }

        fn apply(&self, catalog: ImageCatalog) -> Result<ImageCatalog, DiscoveryError> {
            Ok(catalog)
        }
    }

    struct Second;

    impl FormatHandler for Second {
        fn id(&self) -> &'static str {
            "second"
        }

        fn start(&self, _input: &DiscoveryInput) -> Box<dyn FormatSession> {
            Box::new(SecondSession)
        }
    }

    struct SecondSession;

    impl FormatSession for SecondSession {
        fn start(&mut self, _input: &DiscoveryInput) -> Result<SessionStep, DiscoveryError> {
            Ok(SessionStep::Reject(DiscoveryDiagnostic::from("not used")))
        }

        fn provide(&mut self, _resource: &ResourceOutcome) -> Result<SessionStep, DiscoveryError> {
            Err(DiscoveryError::Session("not used".into()))
        }
    }

    fn metadata_registry() -> Registry {
        let mut registry = Registry::new();
        registry.register_format("need-metadata", Priority(0), Arc::new(NeedMetadata));
        registry
    }

    #[test]
    fn in_memory_discovery_deduplicates_and_finishes() {
        let registry = metadata_registry();
        let mut operation = registry.start("memory://root").unwrap();
        let needs = operation.missing_resources().unwrap();
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].id.0, 0);
        assert_eq!(needs[0].uri, "memory://metadata");

        operation
            .provide(crate::core::discovery::ResourceResponse {
                id: needs[0].id,
                bytes: b"metadata".to_vec(),
                content_type: Some("application/test".into()),
            })
            .unwrap();
        assert!(operation.is_complete());
        assert!(operation.finish().unwrap().is_empty());
    }

    #[test]
    fn operations_have_no_shared_mutable_state() {
        let registry = metadata_registry();
        let mut first = registry.start("memory://root").unwrap();
        let mut second = registry.start("memory://root").unwrap();
        assert_eq!(first.missing_resources().unwrap()[0].id.0, 0);
        assert_eq!(second.missing_resources().unwrap()[0].id.0, 0);

        let first_id = first.missing_resources().unwrap()[0].id;
        first
            .provide(crate::core::discovery::ResourceResponse {
                id: first_id,
                bytes: b"metadata".to_vec(),
                content_type: None,
            })
            .unwrap();
        assert!(first.is_complete());
        assert!(!second.is_complete());
        assert_eq!(second.missing_resources().unwrap().len(), 1);
    }

    #[test]
    fn operation_limits_bound_resources_and_retained_metadata() {
        let registry = metadata_registry();
        let no_resources = DiscoveryLimits {
            max_resources: 0,
            ..DiscoveryLimits::default()
        };
        let mut operation = registry
            .start_with_limits("memory://root", no_resources)
            .unwrap();
        assert_eq!(
            operation.missing_resources(),
            Err(DiscoveryError::ResourceLimitExceeded)
        );

        let one_byte = DiscoveryLimits {
            max_retained_bytes: 1,
            ..DiscoveryLimits::default()
        };
        let mut operation = registry
            .start_with_limits("memory://root", one_byte)
            .unwrap();
        let request = operation.missing_resources().unwrap().remove(0);
        assert_eq!(
            operation.provide(crate::core::ResourceResponse {
                id: request.id,
                bytes: b"too large".to_vec(),
                content_type: None,
            }),
            Err(DiscoveryError::MetadataSizeLimitExceeded)
        );
    }

    #[test]
    fn rule_delegates_to_registered_base_format_and_records_provenance() {
        let mut registry = Registry::new();
        registry.register_format("base", Priority(0), Arc::new(RejectAtRoot));
        registry.register_rule("synthetic-rule", Priority(-1), Arc::new(DelegateRule));
        let mut operation = registry.start("memory://root").unwrap();
        assert!(operation.missing_resources().unwrap().is_empty());
        assert!(operation.is_complete());
        assert!(operation.provenance().iter().any(|event| matches!(
            event,
            ProvenanceEvent::RuleDelegated { rule_id, format_id, uri }
            if rule_id == "synthetic-rule" && format_id == "base" && uri == "memory://delegated"
        )));
    }

    #[test]
    fn profile_is_applied_and_recorded() {
        let mut registry = metadata_registry();
        registry.register_profile("synthetic-profile", Priority(0), Arc::new(RecordingProfile));
        let mut operation = registry.start("memory://root").unwrap();
        let mut requests = operation.missing_resources().unwrap();
        let request = requests.remove(0);
        operation
            .provide(crate::core::discovery::ResourceResponse {
                id: request.id,
                bytes: b"metadata".to_vec(),
                content_type: None,
            })
            .unwrap();
        assert!(operation.provenance().iter().any(|event| matches!(
            event,
            ProvenanceEvent::ProfileApplied { profile_id } if profile_id == "synthetic-profile"
        )));
    }

    #[test]
    fn duplicate_ids_and_priorities_are_rejected() {
        let mut duplicate = Registry::new();
        duplicate.register_format("need-metadata", Priority(0), Arc::new(NeedMetadata));
        duplicate.register_format("need-metadata", Priority(1), Arc::new(NeedMetadata));
        assert!(matches!(
            duplicate.validate(),
            Err(RegistryError::DuplicateId { kind: "format", .. })
        ));

        let mut ambiguous = Registry::new();
        ambiguous.register_format("need-metadata", Priority(0), Arc::new(NeedMetadata));
        ambiguous.register_format("second", Priority(0), Arc::new(Second));
        assert!(matches!(
            ambiguous.validate(),
            Err(RegistryError::AmbiguousPriority { kind: "format", .. })
        ));
    }
}
