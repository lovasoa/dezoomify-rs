//! Stable registration and precedence policy for pure discovery programs.

use std::sync::Arc;

use super::discovery::{DiscoveryInput, DiscoveryLimits, DiscoveryOperation, DiscoveryProgram};
use crate::{
    bulk_text, custom_yaml, dzi, generic, google_arts_and_culture, iiif, iipimage, krpano, nypl,
    zoomify,
};

/// One built-in program: its single name, the URL fragments that identify it
/// for auto-detection, and its constructor.
struct Builtin {
    name: &'static str,
    url_hints: &'static [&'static str],
    construct: fn() -> Arc<dyn DiscoveryProgram>,
}

/// Every built-in program, in recognition order.
///
/// The list order *is* the priority: earlier programs are tried first during
/// auto-detection. Each program has exactly one name, which is used both for
/// diagnostics and as the user-facing selector.
const BUILTINS: &[Builtin] = &[
    Builtin {
        name: "custom",
        url_hints: &[],
        construct: || Arc::new(custom_yaml::CustomDezoomer),
    },
    Builtin {
        name: "google_arts_and_culture",
        url_hints: &[],
        construct: || Arc::new(google_arts_and_culture::GAPDezoomer),
    },
    Builtin {
        name: "zoomify",
        url_hints: &["ImageProperties.xml", "TileGroup"],
        construct: || Arc::new(zoomify::ZoomifyDezoomer),
    },
    Builtin {
        name: "iiif",
        url_hints: &["info.json", "iiif", "manifest.json"],
        construct: || Arc::new(iiif::IiifDezoomer),
    },
    Builtin {
        name: "deepzoom",
        url_hints: &[".dzi", "_files/"],
        construct: || Arc::new(dzi::DziDezoomer),
    },
    Builtin {
        name: "generic",
        url_hints: &["{{"],
        construct: || Arc::new(generic::GenericDezoomer),
    },
    Builtin {
        name: "krpano",
        url_hints: &["tiles.xml"],
        construct: || Arc::new(krpano::KrpanoDezoomer),
    },
    Builtin {
        name: "iipimage",
        url_hints: &["?FIF"],
        construct: || Arc::new(iipimage::IIPImage),
    },
    Builtin {
        name: "nypl",
        url_hints: &["digitalcollections.nypl.org"],
        construct: || Arc::new(nypl::NYPLImage),
    },
    Builtin {
        name: "bulk_text",
        url_hints: &[],
        construct: || Arc::new(bulk_text::BulkTextDezoomer),
    },
];

/// An ordered set of programs to try. Registration order is recognition order.
#[derive(Default, Clone)]
pub struct Registry {
    programs: Vec<(String, Arc<dyn DiscoveryProgram>)>,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one program. Earlier registrations are tried first.
    pub fn register(&mut self, id: impl Into<String>, program: Arc<dyn DiscoveryProgram>) {
        self.programs.push((id.into(), program));
    }

    /// Start independent parser state for every registered program, in order.
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
        DiscoveryOperation::new(&input.into(), self.programs.clone(), limits)
    }
}

/// The name of the first built-in program whose URL hints match `uri`.
fn preferred_name(uri: &str) -> Option<&'static str> {
    BUILTINS
        .iter()
        .find(|builtin| builtin.url_hints.iter().any(|hint| uri.contains(hint)))
        .map(|builtin| builtin.name)
}

/// Compose every built-in program, preferring the one whose URL hints match.
#[must_use]
pub fn default_registry(uri: &str) -> Registry {
    let preferred = preferred_name(uri);
    let mut programs: Vec<(String, Arc<dyn DiscoveryProgram>)> = BUILTINS
        .iter()
        .map(|builtin| (builtin.name.to_owned(), (builtin.construct)()))
        .collect();
    if let Some(name) = preferred
        && let Some(index) = programs.iter().position(|(id, _)| id == name)
    {
        let program = programs.remove(index);
        programs.insert(0, program);
    }
    Registry { programs }
}

/// Resolve a single built-in program by its name.
#[must_use]
pub fn registry_for(name: &str) -> Option<Registry> {
    let builtin = BUILTINS
        .iter()
        .find(|builtin| builtin.name.eq_ignore_ascii_case(name))?;
    let mut registry = Registry::new();
    registry.register(builtin.name, (builtin.construct)());
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
            assert_eq!(registry.programs.len(), 1);
            assert_eq!(registry.programs[0].0, builtin.name);
        }
        assert!(registry_for("nope").is_none());
    }

    #[test]
    fn url_hints_prefer_the_matching_program() {
        assert_eq!(preferred_name("x/info.json"), Some("iiif"));
        assert_eq!(preferred_name("x/unknown"), None);
        assert_eq!(
            default_registry("x/info.json").programs[0].0,
            "iiif",
            "the matching program must be tried first"
        );
    }

    #[test]
    fn default_registry_without_a_hint_keeps_definition_order() {
        assert_eq!(default_registry("x/unknown").programs[0].0, "custom");
        let _ = default_registry("x/unknown").start("memory://root");
    }
}
