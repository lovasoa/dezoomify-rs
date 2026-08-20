//! Application composition of pure discovery programs.

use std::sync::Arc;

use crate::core::{DiscoveryProgram, Priority, Registry};

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
        "custom" => Arc::new(crate::custom_yaml::CustomDezoomer),
        "google_arts_and_culture" => Arc::new(crate::google_arts_and_culture::GAPDezoomer),
        "zoomify" => Arc::new(crate::zoomify::ZoomifyDezoomer),
        "iiif" => Arc::new(crate::iiif::IiifDezoomer),
        "deepzoom" => Arc::new(crate::dzi::DziDezoomer),
        "generic" => Arc::new(crate::generic::GenericDezoomer),
        "krpano" => Arc::new(crate::krpano::KrpanoDezoomer),
        "iipimage" => Arc::new(crate::iipimage::IIPImage),
        "nypl" => Arc::new(crate::nypl::NYPLImage),
        "bulk_text" => Arc::new(crate::bulk_text::BulkTextDezoomer),
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
    use crate::core::discovery::{
        Delegation, DiscoveryEvent, DiscoveryInput, DiscoverySession, DiscoveryStep, Profile,
        ResourceOutcome, ResourceResponse,
    };
    use crate::core::{CatalogEntry, DiscoveryError, StableId};

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

    struct ProfiledDziRule;

    impl DiscoveryProgram for ProfiledDziRule {
        fn start(&self, _: &DiscoveryInput) -> Box<dyn DiscoverySession> {
            Box::new(ProfiledDziRule)
        }
    }

    impl DiscoverySession for ProfiledDziRule {
        fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
            match event {
                DiscoveryEvent::Start => Ok(DiscoveryStep::Delegate(Delegation {
                    program_id: "deepzoom".into(),
                    input: DiscoveryInput::from("memory://deployment/profiled.dzi"),
                    profiles: vec!["repair-dzi".into()],
                })),
                DiscoveryEvent::Resource(_) => unreachable!("rule delegates immediately"),
            }
        }
    }

    struct RepairDzi;

    impl Profile for RepairDzi {
        fn adapt_resource(
            &self,
            outcome: ResourceOutcome,
        ) -> Result<ResourceOutcome, DiscoveryError> {
            Ok(match outcome {
                ResourceOutcome::Response(mut response) => {
                    // A narrowly-scoped deployment repair performed before
                    // the existing DZI parser sees the supplied metadata.
                    response.bytes = br#"<Image TileSize="256" Overlap="0" Format="jpg"><Size Width="500" Height="300"/></Image>"#.to_vec();
                    ResourceOutcome::Response(response)
                }
                failure @ ResourceOutcome::Failure(_) => failure,
            })
        }
    }

    #[test]
    fn profile_customizes_existing_dzi_without_replacing_its_parser() {
        let mut registry = Registry::new();
        registry.register("profiled-dzi-rule", Priority(0), Arc::new(ProfiledDziRule));
        registry.register("deepzoom", Priority(1), Arc::new(crate::dzi::DziDezoomer));
        registry.register_profile("repair-dzi", Arc::new(RepairDzi));

        let mut operation = registry.start("memory://deployment/viewer").unwrap();
        let need = operation
            .missing_resources()
            .unwrap()
            .into_iter()
            .find(|need| need.request.uri.ends_with("profiled.dzi"))
            .expect("delegated DZI metadata request");
        operation
            .provide(ResourceResponse {
                id: need.id,
                bytes: b"deployment-specific broken metadata".to_vec(),
                content_type: Some("application/xml".into()),
            })
            .unwrap();

        let catalog = operation.finish().unwrap();
        let [CatalogEntry::Ready(image)] = catalog.entries() else {
            panic!("the base DZI producer should return one ready image")
        };
        assert_eq!(image.format, StableId::new("deepzoom"));
        assert_eq!(image.levels.last().unwrap().size, Some((500, 300).into()));
        assert!(image.provenance.0.iter().any(|step| {
            step.id == StableId::new("repair-dzi")
                && step.description == "applied discovery profile"
        }));
    }
}
