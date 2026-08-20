use std::{
    fs,
    path::{Path, PathBuf},
};

// Strip test modules, not standalone test-only items embedded in production impls.
fn production_source(source: &str) -> &str {
    source
        .split_once("#[cfg(test)]\nmod ")
        .map_or(source, |(production, _)| production)
}

fn rust_sources(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn stateless_production_sources_do_not_import_runtime_dependencies() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let pure_modules = [
        "core",
        "bulk_text",
        "custom_yaml",
        "dzi",
        "generic",
        "google_arts_and_culture",
        "iiif",
        "iipimage",
        "krpano",
        "nypl",
        "zoomify",
    ];
    let banned = [
        "reqwest",
        "tokio",
        "std::fs::",
        "std::io::",
        "std::path::",
        "use image",
        "image::",
        "clap",
        "indicatif",
        "use log",
        "log::debug!",
        "log::info!",
        "log::warn!",
        "log::error!",
        "log::trace!",
        "debug!(",
        "info!(",
        "warn!(",
        "trace!(",
    ];

    for module in pure_modules {
        let module_root = source_root.join(module);
        let mut files = Vec::new();
        rust_sources(&module_root, &mut files);
        for path in files {
            // Fixtures and unit-test helpers may use application facilities;
            // only the production portion of each module is architecture.
            let source = fs::read_to_string(&path).unwrap();
            let production = production_source(&source);
            for line in production.lines() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for identifier in banned {
                    assert!(
                        !line.contains(identifier),
                        "{} contains forbidden `{identifier}`",
                        path.display()
                    );
                }
            }
        }
    }
}

#[test]
fn architecture_scan_includes_orchestration_after_test_only_items() {
    let source = "#[cfg(test)]\nfn helper() {}\nfn drive() {}\n#[cfg(test)]\nmod tests {}";
    assert!(production_source(source).contains("fn drive()"));
}
