//! Guards the purity contract of this crate's dependency graph.
//!
//! Cargo already enforces the source-level guarantee: a crate absent from this
//! crate's `Cargo.toml` cannot be imported by its source.  Nothing, however,
//! prevents a future edit from adding a runtime crate to `Cargo.toml`.  This
//! test lists the *direct* dependencies that cargo resolved for this crate and
//! fails if any application-runtime crate is among them.
//!
//! Only direct dependencies are checked: pure libraries such as `serde-xml-rs`
//! may legitimately pull `log` into the transitive closure without making it
//! importable by this crate's source.

use std::path::Path;
use std::process::Command;

/// Runtime crates that must never appear in this crate's dependency graph.
/// `log` is a zero-dependency facade (no runtime, no I/O) and is explicitly allowed
/// so dezoomers can emit debug diagnostics when the host initializes a logger.
const BANNED: &[&str] = &[
    "reqwest",
    "tokio",
    "image",
    "image_hasher",
    "clap",
    "indicatif",
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
fn direct_dependencies_contain_no_runtime_crates() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "--manifest-path",
            manifest.to_str().unwrap(),
            "--edges",
            "normal",
            "--depth",
            "1",
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
    // `cargo tree --depth 1` prints this crate itself first, then its direct
    // dependencies, one per line.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    assert!(lines.next().is_some(), "`cargo tree` printed nothing");
    let violations = lines
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| BANNED.contains(name))
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "dezoomify-core must not depend on runtime crates, but its declared \
         dependencies include: {}",
        violations.join(", ")
    );
}
