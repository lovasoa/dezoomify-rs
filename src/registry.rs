//! Application composition of pure discovery programs.

use std::sync::Arc;

use dezoomify_core::core::{DiscoveryProgram, Priority, Registry};

fn preferred_program(uri: &str) -> Option<&'static str> {
    [
        ("info.json", "iiif"),
        ("iiif", "iiif"),
        ("manifest.json", "iiif"),
        (".dzi", "deepzoom"),
        ("_files/", "deepzoom"),
        ("?FIF", "iipimage"),
        ("tiles.xml", "krpano"),
        ("ImageProperties.xml", "zoomify"),
        ("TileGroup", "zoomify"),
        ("digitalcollections.nypl.org", "nypl"),
        ("{{", "generic"),
    ]
    .into_iter()
    .find_map(|(pattern, id)| uri.contains(pattern).then_some(id))
}

fn priority(id: &str, base: i32, preferred: Option<&str>) -> Priority {
    Priority(if preferred == Some(id) { 0 } else { base })
}

const BUILTINS: &[(&str, &str, i32)] = &[
    ("custom", "custom", 10),
    ("google_arts_and_culture", "google_arts_and_culture", 20),
    ("zoomify", "zoomify", 30),
    ("iiif", "iiif", 40),
    ("deepzoom", "deepzoom", 50),
    ("generic", "generic", 60),
    ("krpano", "krpano", 70),
    ("IIPImage", "iipimage", 80),
    ("nypl", "nypl", 90),
    ("bulk_text", "bulk_text", 100),
];

fn program(id: &str) -> Arc<dyn DiscoveryProgram> {
    match id {
        "custom" => Arc::new(dezoomify_core::custom_yaml::CustomDezoomer),
        "google_arts_and_culture" => Arc::new(dezoomify_core::google_arts_and_culture::GAPDezoomer),
        "zoomify" => Arc::new(dezoomify_core::zoomify::ZoomifyDezoomer),
        "iiif" => Arc::new(dezoomify_core::iiif::IiifDezoomer),
        "deepzoom" => Arc::new(dezoomify_core::dzi::DziDezoomer),
        "generic" => Arc::new(dezoomify_core::generic::GenericDezoomer),
        "krpano" => Arc::new(dezoomify_core::krpano::KrpanoDezoomer),
        "iipimage" => Arc::new(dezoomify_core::iipimage::IIPImage),
        "nypl" => Arc::new(dezoomify_core::nypl::NYPLImage),
        "bulk_text" => Arc::new(dezoomify_core::bulk_text::BulkTextDezoomer),
        _ => unreachable!("built-in table contains only known IDs"),
    }
}

fn register(registry: &mut Registry, &(_, id, base): &(&str, &str, i32), preferred: Option<&str>) {
    registry.register(id, priority(id, base, preferred), program(id));
}

/// Compose every built-in producer with the CLI's established URL hints.
pub(crate) fn default_registry(uri: &str) -> Registry {
    let preferred = preferred_program(uri);
    let mut registry = Registry::new();
    for builtin in BUILTINS {
        register(&mut registry, builtin, preferred);
    }
    registry
}

/// Preserve every historical `--dezoomer` spelling while selecting only the
/// corresponding direct producer. `auto` composes the full registry.
pub(crate) fn registry_for_cli(name: &str, uri: &str) -> Option<Registry> {
    if name == "auto" {
        return Some(default_registry(uri));
    }
    let builtin = BUILTINS.iter().find(|(cli_name, _, _)| *cli_name == name)?;
    let mut registry = Registry::new();
    register(&mut registry, builtin, None);
    Some(registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_names_and_url_hints_are_explicit() {
        for name in [
            "auto",
            "custom",
            "google_arts_and_culture",
            "zoomify",
            "iiif",
            "deepzoom",
            "generic",
            "krpano",
            "IIPImage",
            "nypl",
            "bulk_text",
        ] {
            assert!(
                registry_for_cli(name, "https://example.test").is_some(),
                "{name}"
            );
        }
        assert!(registry_for_cli("not-a-format", "https://example.test").is_none());
        assert_eq!(preferred_program("https://x/info.json"), Some("iiif"));
        assert_eq!(preferred_program("https://x/tiles.xml"), Some("krpano"));
        assert_eq!(
            preferred_program("https://x/ImageProperties.xml"),
            Some("zoomify")
        );
        assert_eq!(preferred_program("https://x/unknown"), None);
    }
}
