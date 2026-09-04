# AGENTS.md

dezoomify-rs downloads zoomable images (Zoomify, IIIF, Deep Zoom, krpano,
Google Arts & Culture, IIPImage, WMTS, ArcGIS, generic URL templates, ...)
and reassembles them into a single image file.

## Layout

One workspace, two crates. Dependencies from app to core only.

- `dezoomify-core/` — pure library: turns supplied bytes into image catalogs
  and tile descriptions. No I/O, async, or image decoding; the boundary is
  enforced by [`dezoomify-core/tests/dependency_architecture.rs`](dezoomify-core/tests/dependency_architecture.rs).
- root crate — the native application: fetching, decoding, encoding, CLI.

| Concern | Location |
|---|---|
| CLI arguments | [`src/arguments.rs`](src/arguments.rs) |
| `--dezoomer` name resolution (`auto` vs. named) | [`src/registry.rs`](src/registry.rs) |
| Native discovery driver (HTTP / local files) | [`src/native.rs`](src/native.rs) |
| Fetching, headers, tile cache | [`src/network.rs`](src/network.rs) |
| Tile download loop, progress bars | [`src/download_state.rs`](src/download_state.rs) |
| Orchestration, image/level pickers, bulk mode | [`src/lib.rs`](src/lib.rs) |
| Output encoders (PNG/JPEG/ZIF-TIFF/IIIF) | [`src/encoder/`](src/encoder/) |
| Built-in format registry (names, URL hints, precedence) | [`dezoomify-core/src/core/registry.rs`](dezoomify-core/src/core/registry.rs) |
| Discovery engine (routes, history, limits, request dedup) | [`dezoomify-core/src/core/discovery.rs`](dezoomify-core/src/core/discovery.rs) |
| Catalog, level and tile model | [`dezoomify-core/src/core/model.rs`](dezoomify-core/src/core/model.rs) |
| Known-grid tile plans | [`dezoomify-core/src/core/tile_plan.rs`](dezoomify-core/src/core/tile_plan.rs) |
| Adaptive (probe-driven) tile programs | [`dezoomify-core/src/core/adaptive.rs`](dezoomify-core/src/core/adaptive.rs) |
| Format implementations | [`dezoomify-core/src/<format>/mod.rs`](dezoomify-core/src) |

## Adding a format

1. Declare a co-located [`DezoomerSpec`](dezoomify-core/src/core/discovery.rs).
   Resource-based formats declare one ordered `DiscoveryRoute` table. The core
   acquires the initial URI automatically, dispatches every acquired resource
   through that table, and follows any `DiscoveryStep::Follow` result. Use
   `map_url` for metadata locations derived from an input URL and `extract` for
   ordinary `(uri, bytes) -> ImageCatalog` parsers. Multi-resource handlers
   receive typed resources and return the next declarative action; they never
   implement acquisition states. `immediate` is reserved for URI-only formats
   such as generic URL templates.
2. Fixed grids: construct a validated `Grid` with a `GridRequests` policy
   ([`tile_plan.rs`](dezoomify-core/src/core/tile_plan.rs)). Probe-driven
   sources use `AdaptiveProgram` and `AdaptiveSource` instead.
3. Keep the dezoomer's name and URL hints in that co-located `DezoomerSpec`,
   then list the const in `BUILTINS`
   ([`dezoomify-core/src/core/registry.rs`](dezoomify-core/src/core/registry.rs)).

## Commands

```sh
cargo test --workspace                                  # some tests hit the network
cargo clippy --workspace --all-targets -- -D warnings   # CI gate
cargo fmt
```

## Conventions

- Keep `dezoomify-core` pure: no runtime crates (`reqwest`, `tokio`, `image`,
  filesystem). The `log` facade is allowed.
- Put site-specific discovery code in a separate file and add a passing,
  manually checked `tests/live_dezoomers.rs` case with it.
- CLI `-H` headers must win over format-generated per-request headers
  ([`effective_request_headers`](src/network.rs)).
- Fixtures: `dezoomify-core/testdata/` for core tests, `testdata/` for app
  tests and benches. The root `tiles.yaml` is the published example, fetched
  by a live test — do not delete it.
