# Dezoomer Coverage

This file maps the implemented supported formats to Rust coverage. It is
deliberately separate from the missing-format work in issue #973 section 3.

## Fixture Coverage

The fixture-backed discovery harness in
`dezoomify-core/tests/dezoomer_coverage.rs` covers:

- Generic: padded 2x2 tiles, non-256 tiles, smaller edge tiles, the 999/1000
  boundary, one-dimensional grids, an absent origin tile, 1x1 placeholder
  responses, and encoded braces/query parameters.
- Zoomify: direct metadata, direct tile input, multiple tile groups, and the
  full-resolution-only `NUMTILES` form.
- Deep Zoom: direct XML, PNG/JPEG tile input, sibling descriptor lookup, and
  overlap placement.
- IIIF: Image API 2, Image API 3, malformed `tile_width` fallback, private
  metadata IDs, default-port IDs, Presentation 2 manifest chaining, and
  plain-image-only manifests.
- IIPImage: derived metadata query and row-major `JTL` requests.
- Google Arts & Culture: short-link recognition; the existing module tests
  cover page/tile-info chaining, encrypted tiles, and unencrypted passthrough.
- krpano: the existing module tests cover direct and chained metadata,
  explicit levels, cube faces, scenes, and encrypted XML.

## Supported Format Gaps

These cases use a browser-page adapter. The current Rust core accepts
the corresponding metadata URL, but does not yet scrape these page forms:

- Zoomify: Flash `zoomifyImagePath`, HTML5 `showImage`, Fluid Engage,
  OpenLayers source and tile-source forms, `<url>`, iframe recursion, and the
  University of Bern, Paris, NGV, and Art and Architecture pages.
- Deep Zoom: British Library `Proxy.ashx`, Polona JSON, Paris rewrite, NLA
  `/view`, World Digital Library `view/<group>/<index>`, `zoom.it`,
  `zoomhub.net`, XML/DZI links, and iframe recursion.
- IIIF: Gallica, Van Gogh Micrio, ONB viewer/RepViewer page forms, CONTENTdm
  record pages, Mirador/Universal Viewer manifest parameters, `<micr-io>`,
  National Gallery, London Museum, and Philadelphia page extraction.

These are parity gaps to be implemented as thin deferred adapters, not as
duplicate tile protocols.

## Not In Scope For Step 2

The cases whose expected dezoomer is not present in the Rust
registry remain section 3 work: XLimage, TopViewer/Memorix, FSI, LizardTech,
VLS, Hungaricana, WMTS, ArcGIS MapServer, and pnav. The assembly pixel tests
are application-level coverage rather than dezoomer discovery tests.

## Live Targets

The existing Rust live suite covers the direct protocol targets in
`tests/live_dezoomers.rs`. Additional direct metadata/manifest targets should
be added there only when they do not require an unsupported page adapter. Page
targets above are run manually during the audit and recorded as parity gaps or
external availability failures, rather than making the default test suite
depend on an unimplemented adapter.

### Audit On 2026-08-30

Command: `DEZOOMIFY_LIVE_TESTS=1 cargo test --test live_dezoomers --
--nocapture --test-threads=1`

- 11 of 13 targets passed. CSNTM now passes with the standard `--max-width`
  invocation after correcting scaled IIIF dimensions to round up; the earlier
  `620x655` request returned 404 because the service advertises `620x656`.
- NLS Map View returned HTTP 403 for its first tile because the service rejects
  the advertised full-image tile as exceeding its configured pixel-area
  threshold. This is an external service limitation, not a discovery failure.
- The existing NYPL page target returned no metadata and remains unsuitable as
  a current live smoke target; the NYPL protocol parser is still covered by
  core tests.
- Manual page inputs for NGV, National Gallery, London Museum, Philadelphia,
  NLA, ONB viewer, Paris DZI, and the OpenSeadragon Zoomify example were
  fetched but rejected because the corresponding page adapters are not yet
  implemented.
