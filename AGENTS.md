# AGENTS.md

dezoomify-rs downloads zoomable images (Zoomify, IIIF, Deep Zoom, krpano,
Google Arts & Culture, IIPImage, generic URL templates, ...)
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
| Discovery engine (sessions, limits, request dedup) | [`dezoomify-core/src/core/discovery.rs`](dezoomify-core/src/core/discovery.rs) |
| Catalog, level and tile model | [`dezoomify-core/src/core/model.rs`](dezoomify-core/src/core/model.rs) |
| Known-grid tile plans | [`dezoomify-core/src/core/tile_plan.rs`](dezoomify-core/src/core/tile_plan.rs) |
| Adaptive (probe-driven) tile programs | [`dezoomify-core/src/core/adaptive.rs`](dezoomify-core/src/core/adaptive.rs) |
| Format implementations | [`dezoomify-core/src/<format>/mod.rs`](dezoomify-core/src) |

## Adding a format

1. Implement the [`Dezoomer`](dezoomify-core/src/core/discovery.rs) trait in
   `dezoomify-core/src/<format>/mod.rs` (smallest example:
   [`generic`](dezoomify-core/src/generic/mod.rs)): a `start` constructor from
   `DiscoveryInput` plus an `advance` state machine.
2. Fixed grids: implement `RectangularSource` and wrap it in
   `KnownTilePlan::rectangular` ([`tile_plan.rs`](dezoomify-core/src/core/tile_plan.rs)).
3. Declare the dezoomer's name and URL hints once in a co-located
   `DezoomerSpec` const in the same module, then list that const in `BUILTINS`
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
- CLI `-H` headers must win over format-generated per-request headers
  ([`effective_request_headers`](src/network.rs)).
- Fixtures: `dezoomify-core/testdata/` for core tests, `testdata/` for app
  tests and benches. The root `tiles.yaml` is the published example, fetched
  by a live test — do not delete it.
