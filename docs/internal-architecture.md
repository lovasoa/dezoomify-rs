# Internal architecture

The program is split into two workspace crates: a pure library
(`dezoomify-core`) and the native application (`dezoomify-rs`). This is an
internal boundary: it does not change the command-line interface or the
formats that dezoomify-rs supports.

## Dependency direction

`dezoomify-core` transforms supplied values into catalogs and tile
descriptions. It does not acquire resources, decode pixels, open an output,
schedule tasks, sleep, display progress, or select a filename. In particular,
core modules may not depend on HTTP clients, Tokio, filesystem or path APIs,
image decoders, clap, or terminal progress libraries.

The pure crate's `Cargo.toml` lists only pure libraries, so the compiler and
cargo enforce the boundary: a runtime crate cannot be imported where it is not
a dependency. `dezoomify-core/tests/dependency_architecture.rs` walks the
resolved dependency closure to make a regression in `Cargo.toml` visible.

The native driver owns all of those choices. It pulls a `ResourceNeed`, obtains
the bytes through HTTP, a local-file loader, cache, or another application
mechanism, then calls `DiscoveryOperation::provide` or
`DiscoveryOperation::provide_failure`. It applies a tile's `ProcessingRecipe`,
decodes it when needed, and sends it to an encoder.

```text
core:   input + supplied bytes -> catalog -> tile specifications
native: ResourceNeed -> acquire/retry/cache/decode/write/present
```

## Discovery

`Registry::start` creates a fresh, resumable `DiscoveryOperation`. A registry
can be shared, but parser state, request IDs, supplied metadata, diagnostics,
and provenance belong to that one operation. The application drives it without
an async runtime requirement in core: repeatedly inspect `missing_resources`,
supply each result with `provide` or `provide_failure`, then call `finish`.

Requests are deduplicated by canonical `Request` (URI, ordered headers, and
accepted content types) within one operation. A response fans out to every
candidate waiting for it. Request IDs are stable within an operation and the
registry rejects duplicate stable IDs or ambiguous format/rule precedence.
Candidate-local parse errors reject only that candidate, allowing automatic
discovery to continue.

A catalog may contain `CatalogEntry::Deferred` values. This deliberately
preserves current manifest and bulk behavior: the application can select one
image before resolving its metadata, or expand deferred entries in its bulk
queue. A parent title and provenance travel with the deferred entry; the native
driver may retain a resource cache across child operations.

## Tile programs

A known level uses `KnownTilePlan`: it is immutable, canonical, and replayable.
`KnownTilePlan::rectangular` accepts a `RectangularSource` describing the level
ID, image and tile sizes, overlap, processing recipe, and request generation;
the helper supplies row-major request and placement geometry.

The application can create independent cursors with `plan.cursor()` and pull
bounded batches using `take_ready`. A `TileSpec` states the request, optional
source region, destination origin, optional expected dimensions, and processing recipe; it never
commands the application to fetch or decode.

Adaptive formats use an operation-owned adaptive `TileProgram` instead. The
application requests a bounded ready batch and submits observations keyed by
tile ID. Pure probes use `TileRole::Probe`; `TileRole::ProbeAndOutput` means a
successful probe is also an output tile, while an expected miss is discovery
information rather than a missing output pixel. `TileRole::Output` tiles always
participate in output completeness.

## Extension paths

All registrations have a stable ID and an explicit `Priority` (lower runs
first). The core registry knows nothing about concrete formats. The application
composes built-in programs and applies the CLI's established URL hints.
Registry validation makes duplicate IDs and equal-precedence registrations
actionable errors instead of source-order accidents.

### New format

Implement a pure `DiscoveryProgram` and `DiscoverySession`; parse only bytes supplied
through `ResourceOutcome`, then return an `ImageCatalog`. Fixed-grid formats
should use `KnownTilePlan::rectangular` rather than implement scheduling.
