use std::fs;
use std::sync::Arc;

use dezoomify_rs::core::{
    Dimensions, DiscoveryError, DiscoveryInput, FormatHandler, FormatSession, ImageCatalog,
    KnownTilePlan, Point, Priority, Registry, RequestSpec, ResourceOutcome, ResourcePurpose,
    ResourceRequest, ResourceResponse, SessionStep, StableId,
};

#[test]
fn pure_core_sources_do_not_import_runtime_dependencies() {
    let core_dir = format!("{}/src/core", env!("CARGO_MANIFEST_DIR"));
    let entries = fs::read_dir(core_dir).expect("core directory should be readable in tests");
    let banned_code_fragments = [
        "use reqwest",
        "reqwest::",
        "use tokio",
        "tokio::",
        "use std::fs",
        "std::fs::",
        "use std::io",
        "std::io::",
        "use std::path",
        "std::path::",
        "use image",
        "image::",
        "use clap",
        "clap::",
        "use indicatif",
        "indicatif::",
    ];

    for entry in entries {
        let path = entry.expect("valid core directory entry").path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("core source should be UTF-8");
        for line in source.lines() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for fragment in banned_code_fragments {
                assert!(
                    !code.contains(fragment),
                    "pure core source {} contains forbidden runtime dependency `{fragment}`",
                    path.display(),
                );
            }
        }
    }
}

struct MemoryFormat;

impl FormatHandler for MemoryFormat {
    fn id(&self) -> &'static str {
        "memory-format"
    }

    fn start(&self, _input: &DiscoveryInput) -> Box<dyn FormatSession> {
        Box::new(MemorySession)
    }
}

struct MemorySession;

impl FormatSession for MemorySession {
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
            _ => Err(DiscoveryError::Session(
                "unexpected in-memory resource".into(),
            )),
        }
    }
}

#[test]
fn in_memory_discovery_needs_no_runtime() {
    let mut registry = Registry::new();
    registry.register_format("memory-format", Priority(0), Arc::new(MemoryFormat));
    let mut operation = registry.start("memory://root").unwrap();

    let needs = operation.missing_resources().unwrap();
    assert_eq!(needs.len(), 1);
    operation
        .provide(ResourceResponse {
            id: needs[0].id,
            bytes: b"metadata".to_vec(),
            content_type: None,
        })
        .unwrap();
    assert!(operation.finish().unwrap().is_empty());
}

#[test]
fn known_plan_is_replayable_without_runtime() {
    let plan = KnownTilePlan::rectangular_grid(
        StableId::from("example/full"),
        Dimensions::new(500, 300),
        Dimensions::new(256, 256),
        Point::new(0, 0),
        |Point { x, y }| RequestSpec::new(format!("memory://tiles/{x}_{y}")),
    )
    .unwrap();

    let mut first = plan.cursor();
    let mut second = plan.cursor();
    let first_specs = first.take_ready(16).unwrap().unwrap();
    let second_specs = second.take_ready(16).unwrap().unwrap();
    assert_eq!(first_specs, second_specs);
    assert!(first.take_ready(16).unwrap().is_none());
    assert!(second.take_ready(16).unwrap().is_none());
}
