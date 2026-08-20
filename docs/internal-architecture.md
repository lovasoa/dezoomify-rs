# Internal architecture

The program is split into a pure core and a native application driver. This is
an internal boundary: it does not change the command-line interface or the
formats that dezoomify-rs supports.

## Dependency direction

`src/core` transforms supplied values into catalogs and tile descriptions. It
does not acquire resources, decode pixels, open an output, schedule tasks,
sleep, display progress, or select a filename. In particular, core modules may
not depend on HTTP clients, Tokio, filesystem or path APIs, image decoders,
clap, or terminal progress libraries.

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
an async runtime requirement in core:

```rust,ignore
let mut operation = registry.start(input_uri)?;
while !operation.is_complete() {
    for need in operation.missing_resources()? {
        match application.load(&need).await {
            Ok((bytes, content_type)) => operation.provide(ResourceResponse {
                id: need.id,
                bytes,
                content_type,
            })?,
            Err(error) => operation.provide_failure(ResourceFailure {
                id: need.id,
                message: error.to_string(),
            })?,
        }
    }
}
let catalog = operation.finish()?;
```

Requests are deduplicated by URI and portable request requirements within one
operation. A response fans out to every candidate waiting for it. Request IDs
are stable within an operation and the registry rejects duplicate stable IDs or
ambiguous format/rule precedence.

A catalog may contain `CatalogEntry::Deferred` values. This deliberately
preserves current manifest and bulk behavior: the application can select one
image before resolving its metadata, or expand deferred entries in its bulk
queue. A parent title and provenance travel with the deferred entry; the native
driver may retain a resource cache across child operations.

## Tile programs

A known level uses `KnownTilePlan`: it is immutable, canonical, and replayable.
The rectangular helper supplies row-major request and placement geometry:

```rust,ignore
let plan = KnownTilePlan::rectangular_grid(
    StableId::from("example/full"),
    Dimensions::new(1_000, 700),
    Dimensions::new(256, 256),
    Point::new(0, 0),
    |Point { x, y }| RequestSpec::new(format!("https://tiles.example/{x}_{y}.jpg")),
)?;
```

The application can create independent cursors with `plan.cursor()` and pull
bounded batches using `take_ready`. A `TileSpec` states the request, source and
destination regions, expected dimensions, and processing recipe; it never
commands the application to fetch or decode.

Adaptive formats use an operation-owned adaptive `TileProgram` instead. The
application requests a bounded ready batch and submits observations keyed by
tile ID. Probe tiles are marked `TileRole::Probe`; their expected misses are
discovery information, not missing output pixels. Output tiles use
`TileRole::Output` and determine output completeness.

## Extension paths

All registrations have a stable ID and an explicit `Priority` (lower runs
first). The three extension mechanisms deliberately remain separate.

### 1. New format

Implement a pure `FormatHandler` and `FormatSession`; parse only bytes supplied
through `ResourceOutcome`, then return an `ImageCatalog`. Fixed-grid formats
should use `KnownTilePlan::rectangular_grid` rather than implement scheduling.

```rust,ignore
struct ExampleFormat;

impl FormatHandler for ExampleFormat {
    fn id(&self) -> &'static str { "example" }
    fn start(&self, _: &DiscoveryInput) -> Box<dyn FormatSession> {
        Box::new(ExampleSession)
    }
}

impl FormatSession for ExampleSession {
    fn start(&mut self, _: &DiscoveryInput) -> Result<SessionStep, DiscoveryError> {
        Ok(SessionStep::Need(ResourceRequest::new(
            "memory://example-metadata", ResourcePurpose::Metadata,
        )))
    }
    fn provide(&mut self, metadata: &ResourceOutcome) -> Result<SessionStep, DiscoveryError> {
        let catalog = parse_example(metadata)?; // builds descriptors and a KnownTilePlan
        Ok(SessionStep::Complete(catalog))
    }
}

registry.register_format("example", Priority(20), std::sync::Arc::new(ExampleFormat));
```

### 2. Site-specific discovery for an existing format

A `DiscoveryRule` may parse a supplied page or JSON resource, request one more
resource, then delegate the resulting candidate to an existing format ID. It
must not copy that format's metadata parser or tile logic.

```rust,ignore
// A rule session extracted `info_json` from supplied page bytes.
Ok(SessionStep::Delegate(Delegation {
    format_id: "iiif".into(),
    input: DiscoveryInput::from(info_json),
}))

registry.register_rule("example-site-page", Priority(10), Arc::new(ExampleSiteRule));
```

The operation records `ProvenanceEvent::RuleDelegated`, so callers can explain
how an endpoint was discovered.

### 3. Deployment profile for an existing format

A `Profile` adapts the catalog returned by a base format. Keep the change
narrow: repair a known metadata quirk, adjust a URI or request requirement, or
select a protocol-specific tile recipe. Do not replace the base parser.

```rust,ignore
struct ExampleProfile;
impl Profile for ExampleProfile {
    fn id(&self) -> &'static str { "example-profile" }
    fn apply(&self, catalog: ImageCatalog) -> Result<ImageCatalog, DiscoveryError> {
        Ok(repair_known_deployment_quirk(catalog))
    }
}

registry.register_profile("example-profile", Priority(30), Arc::new(ExampleProfile));
```

Applied profiles are recorded as `ProvenanceEvent::ProfileApplied`. Registry
validation makes duplicate IDs and equal-precedence registrations actionable
errors instead of source-order accidents.
