//! Stable registration and precedence policy for pure discovery programs.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::discovery::{
    DiscoveryError, DiscoveryInput, DiscoveryLimits, DiscoveryOperation, DiscoveryProgram,
};

/// Explicit URL-recognition precedence. Lower values run first.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Priority(pub i32);

#[derive(Clone)]
struct ProgramRegistration {
    id: String,
    priority: Priority,
    program: Arc<dyn DiscoveryProgram>,
}

/// Deterministic registration failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryError {
    DuplicateId {
        kind: &'static str,
        id: String,
    },
    AmbiguousPriority {
        priority: Priority,
        ids: Vec<String>,
    },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateId { kind, id } => write!(f, "duplicate {kind} registration ID '{id}'"),
            Self::AmbiguousPriority { priority, ids } => write!(
                f,
                "ambiguous URL-recognition priority {}: {}",
                priority.0,
                ids.join(", ")
            ),
        }
    }
}

impl std::error::Error for RegistryError {}

/// One immutable program object graph. Programs include both direct formats
/// and discovery rules; a rule delegates rather than requiring a second API.
#[derive(Default, Clone)]
pub struct Registry {
    programs: Vec<ProgramRegistration>,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one format or discovery rule with explicit recognition rank.
    pub fn register(
        &mut self,
        id: impl Into<String>,
        priority: Priority,
        program: Arc<dyn DiscoveryProgram>,
    ) {
        self.programs.push(ProgramRegistration {
            id: id.into(),
            priority,
            program,
        });
    }

    /// Validate all stable identities and auto-recognition priorities.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate IDs or two programs with the same
    /// recognition priority.
    pub fn validate(&self) -> Result<(), RegistryError> {
        let mut program_ids = BTreeSet::new();
        let mut priorities: BTreeMap<Priority, Vec<String>> = BTreeMap::new();
        for registration in &self.programs {
            if !program_ids.insert(registration.id.clone()) {
                return Err(RegistryError::DuplicateId {
                    kind: "program",
                    id: registration.id.clone(),
                });
            }
            priorities
                .entry(registration.priority)
                .or_default()
                .push(registration.id.clone());
        }
        if let Some((priority, mut ids)) = priorities.into_iter().find(|(_, ids)| ids.len() > 1) {
            ids.sort();
            return Err(RegistryError::AmbiguousPriority { priority, ids });
        }
        Ok(())
    }

    /// Start independent parser state for every registered program.
    ///
    /// # Errors
    ///
    /// Returns registration errors before any program executes.
    pub fn start(
        &self,
        input: impl Into<DiscoveryInput>,
    ) -> Result<DiscoveryOperation, RegistryError> {
        self.start_with_limits(input, DiscoveryLimits::default())
    }

    /// Start independent parser state with explicit operation limits.
    ///
    /// # Errors
    ///
    /// Returns registration errors before any program executes.
    pub fn start_with_limits(
        &self,
        input: impl Into<DiscoveryInput>,
        limits: DiscoveryLimits,
    ) -> Result<DiscoveryOperation, RegistryError> {
        self.validate()?;
        let input = input.into();
        let mut programs = self.programs.clone();
        programs.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(DiscoveryOperation::new(
            &input,
            programs
                .into_iter()
                .map(|registered| (registered.id, registered.program))
                .collect(),
            limits,
        ))
    }
}

impl From<RegistryError> for DiscoveryError {
    fn from(error: RegistryError) -> Self {
        Self::Session(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::super::discovery::DiscoverySession;
    use super::*;

    struct NeverStarted;

    impl DiscoveryProgram for NeverStarted {
        fn start(&self, _: &DiscoveryInput) -> Box<dyn DiscoverySession> {
            unreachable!("registry validation must not start programs")
        }
    }

    #[test]
    fn duplicate_program_ids_are_rejected() {
        let mut registry = Registry::new();
        for _ in 0..2 {
            registry.register("same", Priority(0), Arc::new(NeverStarted));
        }
        assert_eq!(
            registry.validate(),
            Err(RegistryError::DuplicateId {
                kind: "program",
                id: "same".into()
            })
        );
    }

    #[test]
    fn ambiguous_program_priorities_are_rejected() {
        let mut registry = Registry::new();
        for id in ["a", "b"] {
            registry.register(id, Priority(7), Arc::new(NeverStarted));
        }
        assert_eq!(
            registry.validate(),
            Err(RegistryError::AmbiguousPriority {
                priority: Priority(7),
                ids: vec!["a".into(), "b".into()],
            })
        );
    }
}
