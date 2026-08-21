//! CLI-facing format registry.
//!
//! The pure core knows every built-in program by name; this module is the only
//! place that interprets the user-facing `--dezoomer` selector (including the
//! `auto` pseudo-name) and maps it onto those programs.

use dezoomify_core::core::{Registry, default_registry, registry_for};

/// Resolve a `--dezoomer` value: `auto` composes every built-in program,
/// otherwise it selects the single named program.
#[must_use]
pub(crate) fn registry_for_cli(name: &str, uri: &str) -> Option<Registry> {
    if name == "auto" {
        Some(default_registry(uri))
    } else {
        registry_for(name)
    }
}
