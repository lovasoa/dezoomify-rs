//! Stable registration and precedence policy for pure dezoomers.

use super::discovery::{DezoomerSpec, DiscoveryInput, DiscoveryLimits, DiscoveryOperation};
use crate::{
    bulk_text, custom_yaml, dzi, generic, google_arts_and_culture, iiif, iipimage, krpano, nypl,
    zoomify,
};

/// Every built-in dezoomer, in recognition order.
///
/// The list order *is* the priority: earlier dezoomers are tried first during
/// auto-detection. Each entry's name and URL hints come from its
/// [`DezoomerMeta`] implementation, so there is no separate per-format
/// declaration to keep in sync.
const BUILTINS: &[DezoomerSpec] = &[
    DezoomerSpec::of::<custom_yaml::Custom>(),
    DezoomerSpec::of::<google_arts_and_culture::Gap>(),
    DezoomerSpec::of::<zoomify::Zoomify>(),
    DezoomerSpec::of::<iiif::Iiif>(),
    DezoomerSpec::of::<dzi::Dzi>(),
    DezoomerSpec::of::<generic::Generic>(),
    DezoomerSpec::of::<krpano::Krpano>(),
    DezoomerSpec::of::<iipimage::Iip>(),
    DezoomerSpec::of::<nypl::Nypl>(),
    DezoomerSpec::of::<bulk_text::BulkText>(),
];

/// An ordered set of dezoomers to try. Registration order is recognition order.
#[derive(Default, Clone)]
pub struct Registry {
    specs: Vec<DezoomerSpec>,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one dezoomer. Earlier registrations are tried first.
    pub fn register(&mut self, spec: DezoomerSpec) {
        self.specs.push(spec);
    }

    /// Start independent parser state for every registered dezoomer, in order.
    #[must_use]
    pub fn start(&self, input: impl Into<DiscoveryInput>) -> DiscoveryOperation {
        self.start_with_limits(input, DiscoveryLimits::default())
    }

    /// Start independent parser state with explicit operation limits.
    #[must_use]
    pub fn start_with_limits(
        &self,
        input: impl Into<DiscoveryInput>,
        limits: DiscoveryLimits,
    ) -> DiscoveryOperation {
        DiscoveryOperation::new(&input.into(), &self.specs, limits)
    }
}

/// The name of the first built-in dezoomer whose URL hints match `uri`.
fn preferred_name(uri: &str) -> Option<&'static DezoomerSpec> {
    BUILTINS
        .iter()
        .find(|spec| spec.url_hints.iter().any(|hint| uri.contains(hint)))
}

/// Compose every built-in dezoomer, preferring the one whose URL hints match.
#[must_use]
pub fn default_registry(uri: &str) -> Registry {
    let preferred = preferred_name(uri);
    let is_other = |d| !preferred.is_some_and(|p| std::ptr::eq(p, *d));
    let others = BUILTINS.iter().filter(is_other);
    let specs = preferred.iter().copied().chain(others).copied().collect();
    Registry { specs }
}

/// Resolve a single built-in dezoomer by its name.
#[must_use]
pub fn registry_for(name: &str) -> Option<Registry> {
    let spec = BUILTINS
        .iter()
        .find(|spec| spec.name.eq_ignore_ascii_case(name))
        .copied()?;
    let mut registry = Registry::new();
    registry.register(spec);
    Some(registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_name_resolves_to_a_single_program() {
        for builtin in BUILTINS {
            let registry = registry_for(builtin.name).unwrap_or_else(|| {
                panic!("built-in `{}` must resolve", builtin.name);
            });
            assert_eq!(registry.specs.len(), 1);
            assert_eq!(registry.specs[0].name, builtin.name);
        }
        assert!(registry_for("nope").is_none());
    }

    #[test]
    fn url_hints_prefer_the_matching_program() {
        assert_eq!(preferred_name("x/info.json").map(|s| s.name), Some("iiif"));
        assert_eq!(preferred_name("x/unknown").map(|s| s.name), None);
        assert_eq!(
            default_registry("x/info.json").specs[0].name,
            "iiif",
            "the matching program must be tried first"
        );
    }

    #[test]
    fn default_registry_without_a_hint_keeps_definition_order() {
        assert_eq!(default_registry("x/unknown").specs[0].name, "custom");
        let _ = default_registry("x/unknown").start("memory://root");
    }
}
