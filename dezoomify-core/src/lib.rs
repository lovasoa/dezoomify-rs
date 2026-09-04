//! Pure discovery core for dezoomify-rs.
//!
//! This crate deliberately contains no HTTP client, async runtime, image
//! decoding, filesystem, or CLI code.  Its modules transform supplied bytes
//! into image catalogs and tile descriptions; an application (such as the
//! `dezoomify-rs` binary) owns fetching, decoding, and writing.  The
//! dependency boundary is enforced by this crate's `Cargo.toml`, which lists
//! only pure libraries, and is regression-tested by
//! `tests/dependency_architecture.rs`.
#![forbid(unsafe_code)]
#![deny(clippy::cognitive_complexity)]
#![deny(clippy::too_many_lines)]
#![deny(clippy::pedantic)]
// The discovery API is documented for readers, but does not yet annotate every
// `Result`-returning function with `# Errors` and `# Panics` sections.
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]

pub mod arcgis;
pub mod bulk_text;
pub mod core;
pub mod custom_yaml;
pub mod dzi;
pub mod fsi;
pub mod generic;
pub mod google_arts_and_culture;
pub mod hungaricana;
pub mod iiif;
pub mod iipimage;
pub mod krpano;
pub mod lizardtech;
pub mod pnav;
pub mod topviewer;
pub mod vec2d;
pub mod vls;
pub mod wmts;
pub mod xlimage;
pub mod zoomify;

mod json_utils;
mod template;
mod web_page;

pub use vec2d::Vec2d;

/// Browser-like headers sent by default with every request, both by the
/// application's HTTP client and by `custom_yaml` tile requests.
///
/// # Panics
///
/// Panics if the bundled `default_headers.yaml` fails to parse, which would be
/// a bug in this crate.
#[must_use]
pub fn default_headers() -> std::collections::HashMap<String, String> {
    serde_yaml::from_str(include_str!("default_headers.yaml"))
        .expect("bundled default headers must be valid YAML")
}
