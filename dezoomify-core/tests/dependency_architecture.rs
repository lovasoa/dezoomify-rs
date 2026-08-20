//! Guards the purity contract of this crate's dependency graph.
//!
//! Cargo already enforces the source-level guarantee: a crate absent from
//! `Cargo.toml` cannot be imported.  Nothing, however, prevents a future edit
//! from adding a runtime crate to `Cargo.toml`.  This test walks the actual
//! dependency closure that cargo resolved for this crate and fails if any
//! application-runtime crate is reachable.

use std::path::Path;
use std::process::Command;

/// Runtime crates that must never appear in this crate's dependency closure.
const BANNED: &[&str] = &[
    "reqwest",
    "tokio",
    "image",
    "image_hasher",
    "clap",
    "indicatif",
    "log",
    "env_logger",
    "human-panic",
    "colour",
    "png",
    "zif-tiff",
    "futures",
    "tempfile",
    "criterion",
    "sanitize-filename-reader-friendly",
];

#[test]
fn dependency_closure_contains_no_runtime_crates() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "--manifest-path",
            manifest.to_str().unwrap(),
            "--edges",
            "normal,build",
            "--prefix",
            "none",
        ])
        .output()
        .expect("failed to run `cargo tree`");
    assert!(
        output.status.success(),
        "`cargo tree` failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let closure = String::from_utf8_lossy(&output.stdout);
    let violations = closure
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| BANNED.contains(name))
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "dezoomify-core must not depend on runtime crates, but its dependency closure \
         contains: {}",
        violations.join(", ")
    );
}
