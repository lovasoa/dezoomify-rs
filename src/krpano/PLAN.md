# Encrypted krpano XML support plan

## Goal

Add support for krpano XML files whose top-level content is an `<encrypted>` element. The final implementation must decrypt and decode those files into normal krpano XML, then feed the plaintext into the existing krpano metadata parser unchanged.

This plan is the working source of truth. When new facts are learned, update the relevant "Known facts", "Open questions", and "Action plan" sections in the same change.

## Target Code Structure

After all phases, the code should live in an `encrypted/` module directory under `src/krpano/`:

```
src/krpano/
├── mod.rs              # KrpanoDezoomer, Level, load_from_properties (unchanged)
├── krpano_metadata.rs  # XML metadata deserialization (unchanged)
├── encrypted/
│   ├── mod.rs          # Re-exports, EncryptedKrpanoError, is_encrypted_xml,
│   │                   #   encrypted_payload, decrypt_xml (branch dispatch)
│   ├── header.rs       # KencHeader, KencBranch enum, branch classification
│   ├── codecs.rs       # decode_modified_base85, lz4_decompress_block,
│   │                   #   decode_packed_viewer_js_payload
│   ├── crypto.rs       # decrypt_bytes (RC4-like)
│   ├── viewer.rs       # extract_key_from_viewer_js, extract_decoded_viewer_js,
│   │                   #   next_js_string_literal, looks_like_* helpers
│   ├── old_engine.rs   # Old engine license key derivation, old Z pipeline assembly
│   ├── modern_engine.rs# Startup key-unpack IIFE extraction, we.subdiv row reads,
│   │                   #   static constant resolution
│   └── branches.rs     # Branch transform dispatch: Z, P/P, R/R body transforms
└── PLAN.md
```

**Incremental split plan:**
1. After Phase 2: move `encrypted.rs` → `encrypted/` module; split into `mod.rs` (public API + errors), `header.rs`, `codecs.rs`, `crypto.rs`, `viewer.rs`.
2. Phase 3: add `old_engine.rs`.
3. Phase 4: add `modern_engine.rs`.
4. Phases 5–6: add `branches.rs`.
5. Phase 7: complete `decrypt_xml` in `mod.rs`, wiring all branch modules together.

**Rationale:**
- Each module has a single, well-scoped responsibility.
- Tests stay co-located in `#[cfg(test)] mod tests` blocks within each module.
- The split is done incrementally so no commit contains both a refactor and new logic.
- No module exceeds ~300 lines, keeping code reviewable.

## Current Code Status

| Area | Current state | Next action |
| --- | --- | --- |
| Encrypted XML detection | Implemented: detects `<encrypted>` and concatenates split CDATA payloads. | Keep covered by tests. |
| `KENC....` header parser | Implemented: parses the eight-byte marker and reports unknown headers precisely. | Add exact fixture-header tests for `KENCRUZR`, `KENCPUZR`, and `KENCRURR`; add branch classification. |
| Packed viewer JS extraction | Implemented: modified Base85 + LZ4 decoder extracts raw engine source from all current viewer fixtures. | Keep as the source for engine analysis and key extraction. |
| Modified Base85 codec | Implemented and unit-tested. | Use for `Z` branch payload decoding. |
| LZ4 block codec | Implemented and unit-tested. | Use for `Z` branch after byte decryption. |
| Base64 payload codec | Not implemented for XML payloads. | Implement only when a `B` branch fixture or test vector exists. |
| Byte decryptor | Implemented as an RC4-like helper with synthetic round-trip coverage. | Add fixture vectors using real derived keys. |
| Wrapper `krp:` key extraction | Implemented as a simple viewer JS regex. | Verify against all fixtures; make errors precise. |
| Modern static string/key extraction | Partially understood in investigation: the wrapper key unpacks into `we.subdiv` rows without executing viewer JS; the modern default byte-helper key resolves to `actions overflow` for all current modern fixtures. | Port the wrapper-key unpacker plus direct `we.subdiv` row reads; defer the stateful widened-key branch until needed by a fixture. |
| Old license key derivation | Not implemented. | Port the minimal license-decoder path that reaches the old key assignment. |
| `decrypt_xml` | Still a stub. | Wire after branch transforms and key derivation have fixture vectors; `2018-04-04` now has a proven modern `Z` pipeline vector from temporary analysis. |

## Fixture Corpus

All checked-in encrypted fixtures live under [testdata/krpano/encrypted](../../testdata/krpano/encrypted/). All viewer JS fixtures currently decode cleanly with `extract_decoded_viewer_js`. Only `2023-04-30/decoded.js` is checked in; the other decoded engines were materialized temporarily under `/tmp/krpano_decoded` during analysis.

| Fixture | Viewer file | XML header | Branch family | Decoded engine bytes | Wrapper `krp:` length | Decoded engine traits |
| --- | --- | --- | --- | ---: | ---: | --- |
| `old` | `krpano.js` | `KENCRUZR` | old `Z` | 214903 | 136 | Literal `KENC`, old numeric `_[]`, no `decryptData`. |
| `2015-08-04` | `tour.js` | `KENCRUZR` | old `Z` | 191689 | 60 | Literal `KENC`, old numeric `_[]`, no `decryptData`, includes `G` mode code. |
| `2017-09-21` | `tour.js` | `KENCRUZR` | old `Z` | 227010 | 115 | Literal `KENC`, old numeric `_[]`, no `decryptData`. |
| `2018-04-04` | `tour.js` | `KENCPUZR` | modern `Z` | 254751 | 17 | No literal `KENC`; startup rebinds `_` to `we.subdiv`; default key resolves to `actions overflow`. |
| `2023-02-07` | `tour.js` | `KENCRURR` | modern `R/R` | 359957 | 29 | No literal `KENC`; startup rebinds `_` to `we.subdiv`; replacement token resolves to `z`. |
| `2023-04-30` | `tour.js` | `KENCRURR` | modern `R/R` | 441405 | 110 | No literal `KENC`; startup rebinds `_` to `we.subdiv`; replacement token resolves to `z`. |
| `2023-12-11` | `tour.js` | `KENCRURR` | modern `R/R` | 441589 | 163 | No literal `KENC`; startup rebinds `_` to `we.subdiv`; replacement token resolves to `z`. |
| `2024-12-20` | `tour.js` | `KENCRURR` | modern `R/R` | 482960 | 45 | No literal `KENC`; startup rebinds `_` to `we.subdiv`; replacement token resolves to `z`. |

## Known Facts

### Encrypted XML wrapper

- Encrypted XML payloads are wrapped in `<encrypted>`.
- Payload text may be split across multiple CDATA sections; all CDATA chunks must be concatenated before header parsing.
- The first eight payload bytes are the `KENC....` header.
- Unsupported or malformed headers must fail with the exact header bytes in the error.

### Packed viewer loader

- The outer viewer-loader obfuscation is modified Base85 followed by LZ4 block decompression.
- The decoded result is raw krpano engine JS that the original loader executes with `new Function(...)`.
- The wrapper `krp:` key exists in the packed viewer wrapper, not in the decoded engine source.
- A direct Node VM attempt to evaluate `2023-04-30/decoded.js` failed because the decoded source expects the loader invocation environment and uses top-level `arguments`. Do not build the Rust implementation around executing arbitrary viewer JS.

### Header arithmetic

Modern engines derive branch values with `r=4` and `k=(r<<4)+(r<<2)=80`.

| Header byte | Decode rule | Observed values | Meaning known so far |
| --- | --- | --- | --- |
| byte 4 | `(charCode - 80) >> 1` | `P => 0`, `R => 1` | Mode/key-policy value. Semantic names still need confirmation. |
| byte 5 | direct character check | `U` | Required by all observed branches. |
| byte 6 | `charCode - 80` | `B => -14`, `P => 0`, `R => 2`, `Z => 10` | Selects body transform branch. |
| byte 7 | direct character check | `R` | Final marker in all current fixtures. |

Branch table:

| Header | Mode value | Byte 6 value | Branch status |
| --- | ---: | ---: | --- |
| `KENCRUZR` | 1 | 10 | `Z`: modified Base85, byte decrypt, LZ4, UTF-8. Used by old fixtures. |
| `KENCPUZR` | 0 | 10 | `Z`: modified Base85, byte decrypt, LZ4, UTF-8. Used by `2018-04-04`. |
| `KENCPUPR` | 0 | 0 | `P/P`: accepted by modern code, exact transform pending. |
| `KENCRURR` | 1 | 2 | `R/R`: accepted by modern code, exact transform pending. Used by 2023+ fixtures. |
| `KENC..B.` | varies | -14 | `B`: Base64, byte decrypt, UTF-8 according to decoded code, but no current fixture exercises it. |

Important correction: `KENCRURR` does not use the `Z` Base85 + LZ4 branch.

### Byte decryptor

- The byte decryptor is RC4-like and already ported as `decrypt_bytes`.
- The common prefix length is 128 bytes.
- The encrypted body start offset is `128 + (input[65] & 7)`.
- In modern engines the `65` index is obfuscated through a browser-name array, but currently resolves to `"Android Browser".charCodeAt(0)`.
- Old literal-header engines use the same broad helper shape: interleave the first 128 encrypted bytes with key bytes, initialize pseudo-random state, then decrypt from the offset above.
- Modern helpers use a 15-byte key mask for the default key and a 127-byte key mask for the widened-key path.

Modern key expressions found so far:

| Fixture | Default-key expression | Widened-key expression |
| --- | --- | --- |
| `2018-04-04` | `nc(_(12931))` | `_(26890,1)` |
| `2023-02-07` | `mc(_(1502))` | `_(8247,1)` |
| `2023-04-30` | `gd(_(5697))` | `_(9525,1)` |
| `2023-12-11` | `hd(_(5761))` | `_(1783,1)` |
| `2024-12-20` | `cd(_(360))` | `_(3255,1)` |

Temporary static probes on 2026-06-26 resolved every default-key expression above through the wrapper-unpacked `we.subdiv` rows. All current modern fixtures produce the same 16-byte default key string: `actions overflow`.

The widened-key expressions above do not resolve as direct rows. They enter a stateful `we.subdiv` branch and require replaying relevant `we.subdiv` calls in source/runtime order. No current successful fixture path has proven that widened key is needed:

- `2018-04-04` has mode value `0`, so the modern `Z` byte helper uses the default key and decrypts successfully.
- Current `KENCRURR` fixtures enter the replacement branch before the byte helper, so they do not use either byte-helper key in that branch.

Modern `Z` vector proven by `/tmp/krpano_decrypt_2018_probe.js`:

| Fixture | Header | Default key | Base85 bytes | Post-byte-decrypt bytes | Plaintext bytes | Plaintext prefix |
| --- | --- | --- | ---: | ---: | ---: | --- |
| `2018-04-04` | `KENCPUZR` | `actions overflow` | 7932 | 7803 | 36407 | whitespace then `<krpano>` |

### Old engine family

- Applies to `old`, `2015-08-04`, and `2017-09-21`.
- Decoded engines contain literal `KENC` branch logic.
- They use an old numeric `_[]` string table and do not use the modern `decryptData` constant system.
- `old` and `2017-09-21` require a license-derived key for `R` mode. If the key variable is absent (`Pd` in `old`, `pe` in `2017-09-21`), the helper returns `null`.
- `2015-08-04` accepts `P`, `R`, and an embedded-key `G` mode. Its `R` branch can use license-derived key variable `od`, but also has a default-key fallback not seen in later old engines.
- The old license decoder has a `case 7` branch that pads the recovered key string to 128 characters when needed, then stores it in the helper key variable (`od`, `pe`, or `Pd` depending on version).
- The `decodeLicense=function(a){return null}` found inside the resource module is not the old license decoder that assigns those variables.

### Modern engine family

- Applies to `2018-04-04` and newer fixtures.
- Decoded engines do not contain literal `KENC`; the resource-loader branch constants are reconstructed through startup-unpacked `we.subdiv` rows, with the initial `_()` / `decryptData(...)` wrapper retained as `Rt` for secondary stateful branches.
- Important correction from 2026-06-26: `_` is only briefly the `decodeLicense(...)` / `decryptData(...)` wrapper. A startup key-unpack IIFE evaluates a generated string equivalent to `Rt=_;_=arguments[2]`, where `arguments[2]` is the `we.subdiv` function passed to the eval call. After that point, most source-level `_("<id>")` calls are calls into `we.subdiv`, and the old wrapper is saved as `Rt`.
- Example wrappers:
  - `2018-04-04`: `_=function(a,b){return a?(pa.decodeLicense(b),pa.decryptData(a)):eval(b)}`.
  - `2023-04-30`: `_=function(l,a){return l?(ra.decodeLicense(a),ra.decryptData(l)):eval(a)}`.
- The resource module's modern `decodeLicense` body has the shape `decodeLicense=function(x){ accumulator += x; return null }`.
- The modern `decryptData` body repeatedly Base64-decodes and UTF-8-decodes an expression, then feeds generated `_("<id>","<license-fragment>")` calls back through the same accumulator until a `#` sentinel is reached.
- The decrypted constant owner name changes across builds (`pa`, `ra`, `la`, etc.). Extraction must use structure, not minified object names.

### Modern static extraction facts

Temporary probes created under `/tmp` on 2026-06-26:

- `/tmp/krpano_extract_decoded.js`: reimplemented the packed-viewer Base85 + LZ4 decoder and materialized decoded engines under `/tmp/krpano_decoded`.
- `/tmp/krpano_static_probe.js`: parsed source text, extracted the startup key-unpack IIFE, computed its checksum, and unpacked the outer wrapper `krp:` key into `we.subdiv` row data without evaluating decoded viewer JS.
- `/tmp/krpano_decrypt_2018_probe.js`: used the statically extracted default key to prove the `2018-04-04` modern `Z` decrypt path.

The startup key-unpack IIFE is structurally stable but has three observed subfamilies:

| Fixture family | Wrapper-key input | Checksum constant | Observed result |
| --- | --- | ---: | --- |
| `2018-04-04` | second `embedpano` parameter (`ue`) | 22248 | checksum result gives `n=32`, `q=3`, final generated char `_`. |
| `2023-02-07` | third-or-second `embedpano` parameter (`xe || Vd`) | 22557 | checksum result gives `n=32`, `q=3`, final generated char `_`. |
| `2023-04-30` and newer | top-level `arguments[2] || arguments[1]` | 23293 | checksum result gives `n=32`, `q=3`, final generated char `_`. |

The helper named `Xa`, `ua`, `Wa`, `Za`, `yb`, etc. is a deterministic computed alias for `String.fromCharCode`, derived from the stable `Ma`/browser-name array. It should be reduced structurally instead of name-matched.

The `we.subdiv(0, rows, side)` startup call only installs wrapper-derived arrays into the `we.subdiv` closure and nulls branch `0`. Direct `_()` row reads then work as follows:

- If `id & 1`, direct row index is `id >> 7` and branch is `(id >> 2) & 15`.
- Otherwise, direct row index is `(id >> 2) & 255` and branch is `(id >> 11) & 15`.
- Branch `0` returns the row as a string after applying the current row offset.

Current direct-row constants of interest:

| Purpose | IDs by fixture | Resolved value |
| --- | --- | --- |
| Modern default byte-helper key | `12931`, `1502`, `5697`, `5761`, `360` | `actions overflow` |
| `P/P` and `R/R` replacement token | `3139`, `6721`, `1420`, `7875`, `152` | `z` |

### Modern branch transform facts

The modern resource-file function has the same high-level branch structure across current modern fixtures:

- It checks the first 4 bytes against the direct-row constant `KENC`.
- It derives the mode value from header byte 4 and the transform value from byte 6 using the Base64-table indices already documented above.
- For the `Z` branch (`byte6 == 10`), it decodes modified Base85, calls the byte helper with the mode value as the widened-key flag, LZ4-decompresses the decrypted block, then UTF-8-decodes it.
- For the `P/P` and `R/R` branch (`byte6 == 2 * mode_value`), it currently performs only `body.replaceAll("z", "\\")` in the resource-loader branch. This does not yet explain why checked-in `KENCRURR` bodies become plaintext XML; either another downstream interpretation step remains, or the exact call path differs after this return.
- For the `B` branch (`byte6 == -14`), it Base64-decodes, byte-decrypts, and UTF-8-decodes according to the same resource function, but no current fixture covers it.

## Decisions

- Do not execute arbitrary decoded viewer JS in Rust.
- Do decode packed viewer JS and statically extract only the data needed for XML decryption.
- Do not depend on minified names such as `w`, `b`, `ra`, `pa`, or `la`; they change across versions.
- Split implementation into two known engine families:
  - old literal-header engines,
  - modern startup-unpack / `we.subdiv` engines with `decryptData` retained as a secondary helper behind `Rt`.
- Treat `P/P` and `R/R` as their own body-transform family until proven otherwise.
- Keep unobserved branches (`B`, 2015 `G`) unsupported or fixture-gated until test data exists.
- Wire `decrypt_xml` only after each stage has fixture-driven intermediate vectors.
- Do not pursue keyless known-plaintext RC4 attacks: the per-file keystream (variable 128-byte prefix) and sparse known plaintext make this infeasible (see analysis above). Key extraction from viewer JS is the correct path.

## Open Questions

Each item below needs an explicit answer, fixture vector, or implementation decision before encrypted XML support is considered complete.

| ID | Question | Blocks | Required proof |
| --- | --- | --- | --- |
| Q1 | What exactly completes the `P/P` and `R/R` body transform after `body.replaceAll("z", "\\")`? | 2023+ fixtures, HIROX-style `KENCRURR`, generated `KENCPUPR`. | Trace the downstream consumer after the replacement branch, identify whether another escape/parser/decode step runs, and produce plaintext vectors. |
| Q2 | When is the modern widened byte-helper key actually needed, and how should its stateful `we.subdiv` branch be replayed? | Future modern mode-1 `Z`/`B` fixtures; not the current `2018-04-04` `Z` path. | Port the relevant `we.subdiv` stateful branches or prove no supported fixture needs them; assert default key `actions overflow` for all current modern fixtures. |
| Q3 | How is the old license key derived from wrapper data? | `KENCRUZR` fixtures in `old`, `2015-08-04`, `2017-09-21`. | Port or reproduce the license-decoder path that assigns `od`, `pe`, or `Pd`; verify final key strings and decrypted XML. |
| Q4 | What are the semantic names for header modes? | Documentation quality, future compatibility. | Confirm which combinations map to public-key, protected/license-key, custom-key, compressed, and uncompressed krpano modes. |
| Q5 | Should `B` and `G` branches be implemented now? | Only future/unobserved fixtures. | Add fixture vectors or explicitly keep them as precise unsupported errors. |
| Q6 | Can one modern structural extractor cover future modern builds? | Robustness beyond current fixtures. | Tests over all modern fixtures and fallback errors that name the missing structural anchor. |
| Q7 | How should viewer JS/key data enter `KrpanoDezoomer`? | User-facing integration. | Decide between `NeedsData`, URL inference, explicit key bundle input, or a combined approach; add tests for the chosen flow. |

## Action Plan

### Phase 1: Freeze Fixture Metadata

Status: next.

Goal: make the current corpus facts executable so regressions are caught immediately.

Tasks:

1. Add a fixture metadata test that reads every encrypted XML fixture and asserts its `KENC....` header.
2. Add a viewer-wrapper test that asserts the extracted `krp:` key length for each fixture.
3. Add a decoded-viewer test that asserts decoded engine byte length for each fixture, with small tolerance only if the fixture changes intentionally.
4. Update `KencHeader` tests to include `KENCRUZR`, `KENCPUZR`, and `KENCRURR`.
5. Add a `KencBranch` or equivalent classification API for known branch values.

Acceptance checks:

- The current corpus table above can be regenerated from tests.
- Unknown headers still fail with exact header bytes.
- `KENCRURR` is classified as `R/R`, not `Z`.

### Phase 2: Build an Analysis Harness

Status: not started.

Goal: make intermediate decrypt stages inspectable without committing large decoded JS files.

Tasks:

1. Add test-only helpers or ignored diagnostics that can materialize:
   - wrapper `krp:` key,
   - decoded engine JS,
   - encrypted XML payload body,
   - body after branch body-decoding,
   - body after byte decryption,
   - body after LZ4 for `Z`.
2. Keep generated decoded JS under `/tmp` or test output, not committed, unless a small fixture-specific excerpt is needed.
3. Make diagnostics print fixture name, header, branch, and byte lengths at every stage.

Acceptance checks:

- Running the harness over the corpus produces a stable stage table.
- The harness can isolate which stage fails for each fixture.

### Phase 3: Derive Old-Engine Keys

Status: not started.

Goal: decrypt the `KENCRUZR` fixtures from the old engine family.

Tasks:

1. Locate the real old license decoder in `old`, `2015-08-04`, and `2017-09-21`.
2. Port only the path needed to process the wrapper `krp:` key and reach `case 7`.
3. Verify the assigned key variable:
   - `old`: `Pd`,
   - `2015-08-04`: `od`,
   - `2017-09-21`: `pe`.
4. Feed the derived key into the existing `decrypt_bytes` helper.
5. Decode the `Z` branch body with modified Base85, byte decrypt, LZ4, UTF-8.

Acceptance checks:

- Each old fixture has tests for derived key length/content hash.
- Each old fixture has tests for post-byte-decrypt length/hash.
- Each old fixture decrypts to plaintext XML.
- Unsupported 2015 `G` mode has a precise fixture-gated error unless implemented.

### Phase 4: Derive Modern Static Constants and Keys

Status: partially proven in temporary probes.

Goal: resolve modern startup-unpacked `we.subdiv` constants into concrete strings without executing arbitrary viewer JS.

Tasks:

1. Extract the wrapper `krp:` key before decoding the engine.
2. Find the modern `_()` wrapper structurally, then find the startup key-unpack IIFE that rebinds `_` to `we.subdiv`.
3. Implement the source-text helpers needed by the startup unpacker:
   - balanced function-body extraction,
   - `Rd`-style body normalization,
   - `qf` checksum,
   - `Lf` permutation table,
   - `krp:` payload unpack into row and side arrays.
4. Reduce the computed `Xa`/`ua`/`Wa`/`Za`/`yb` helper to `String.fromCharCode`.
5. Implement direct `we.subdiv` branch-0 row reads.
6. Assert direct modern constants:
   - default byte-helper key: `actions overflow`,
   - replacement token for `P/P` and `R/R`: `z`,
   - `KENC`, header constants, and resource branch string constants needed by branch dispatch.
7. Defer or separately implement stateful `we.subdiv` branches for widened-key ids (`_(26890,1)`, `_(8247,1)`, `_(9525,1)`, `_(1783,1)`, `_(3255,1)`) only when a supported fixture path needs them.
8. Keep `decodeLicense` / `decryptData` support scoped to `Rt` calls emitted by stateful `we.subdiv` branches; do not make it the primary modern extractor.

Acceptance checks:

- Modern startup unpacking works for `2018-04-04`, `2023-02-07`, `2023-04-30`, `2023-12-11`, and `2024-12-20`.
- Direct constants match the temporary probe values, including `actions overflow` and replacement token `z`.
- Failures report which structural anchor was missing.
- No code path evaluates decoded viewer JS directly.

### Phase 5: Finish the `Z` Branch

Status: partially proven for modern `2018-04-04`; still blocked on old-engine keys for old fixtures.

Goal: prove the already-ported codecs and byte decryptor against real `Z` fixtures.

Tasks:

1. Promote the `2018-04-04` (`KENCPUZR`) temporary vector to tests using default key `actions overflow`:
   - modified Base85,
   - byte decrypt with widened-key flag `false`,
   - LZ4 using the eight-byte little-endian block header,
   - UTF-8.
2. For old `KENCRUZR` fixtures, use old derived keys and the same `Z` pipeline.
3. Add intermediate vectors for body-decoded bytes, post-byte-decrypt bytes, post-LZ4 bytes, and plaintext XML.

Acceptance checks:

- `2018-04-04` decrypts to valid krpano XML.
- `2018-04-04` stage lengths match the temporary vector: 7932 Base85 bytes, 7803 post-byte-decrypt bytes, 36407 plaintext bytes.
- `old`, `2015-08-04`, and `2017-09-21` decrypt to valid krpano XML.
- The existing plaintext krpano parser consumes decrypted XML without changes.

### Phase 6: Resolve `P/P` and `R/R`

Status: narrowed by static branch trace; plaintext step still unresolved.

Goal: decrypt current modern `KENCRURR` fixtures and generated/public `KENCPUPR` fixtures.

Tasks:

1. Port the currently traced branch as `body.replaceAll("z", "\\")`.
2. Add intermediate vectors for every `KENCRURR` fixture before and after the replacement.
3. Trace the downstream consumer after this branch returns; current post-replacement output is not plaintext XML.
4. Identify whether another escape parser, generated action decoder, XML parser path, or call-path distinction completes plaintext recovery.
5. Add `KENCPUPR` vectors if the generated public fixture is license-safe and small enough.

Acceptance checks:

- `2023-02-07`, `2023-04-30`, `2023-12-11`, and `2024-12-20` decrypt to valid krpano XML.
- The branch implementation is selected by parsed header values, not by fixture name.
- `KENCRURR` never falls through to the `Z` pipeline.

### Phase 7: Wire `decrypt_xml`

Status: blocked on at least one full fixture path.

Goal: replace the stub with a staged, testable decrypt function.

Tasks:

1. Define internal structs for extracted viewer decryption data:
   - engine family,
   - wrapper key,
   - derived byte-helper keys,
   - supported branch transforms.
2. Keep XML payload extraction separate from viewer key extraction.
3. Implement branch dispatch from `KencHeader` / `KencBranch`.
4. Return precise errors for:
   - missing viewer data,
   - unsupported branch,
   - missing structural anchor,
   - bad body codec,
   - byte decrypt failure,
   - decompression failure,
   - non-UTF-8 output.
5. Feed decrypted XML into the existing krpano parser unchanged.

Acceptance checks:

- Unit tests cover each successful branch that has fixtures.
- Unit tests cover unsupported `B`, `G`, or unknown headers if not implemented.
- No plaintext krpano behavior regresses.

### Phase 8: Integrate with `KrpanoDezoomer`

Status: not started.

Goal: make encrypted XML work in the normal dezoomer flow without hard-coding one site layout.

Tasks:

1. Decide how viewer JS is discovered or supplied:
   - use `DezoomerError::NeedsData` for viewer JS,
   - infer common `tour.js` / `krpano.js` URLs when safe,
   - allow pre-supplied viewer/key data,
   - or combine these.
2. Extend `KrpanoDezoomer` state only as much as needed to hold pending encrypted XML and viewer JS.
3. Add integration tests for encrypted XML requiring a second resource.
4. Ensure plaintext XML still takes the existing fast path.

Acceptance checks:

- A local encrypted fixture produces the same tile URL metadata as its plaintext XML.
- Missing viewer JS produces an actionable `NeedsData` or equivalent error.
- Plaintext krpano tests still pass.

### Phase 9: End-to-End Validation

Status: not started.

Goal: prove the implementation against fixtures and the motivating HIROX-style capture.

Tasks:

1. Add unit tests for each stage and branch.
2. Add local dezooming tests comparing encrypted fixture output to plaintext output.
3. Run existing krpano tests.
4. Test the HIROX capture end to end.
5. Document any unsupported branch with exact error behavior.

Acceptance checks:

- Every current fixture either decrypts successfully or is explicitly marked unsupported with a precise reason.
- The HIROX capture produces expected levels and tile URL structure.
- No plaintext support regresses.

## Immediate Next Work

Do these in order:

1. Add fixture metadata tests and `KencBranch` classification.
2. Build the analysis harness for intermediate vectors.
3. Port the modern startup-unpack and direct `we.subdiv` row-read extractor.
4. Promote the `2018-04-04` full modern `Z` vector to Rust tests and implementation.
5. Then resolve old `KENCRUZR` key derivation and the downstream `KENCRURR` post-replacement decode path in parallel if useful.

## Commit Strategy

- Commit 1: plan and fixture metadata tests.
- Commit 2: `KencBranch` classification and exact header coverage.
- Commit 3: analysis harness and intermediate vector tests.
- Commit 4: modern startup-unpack / direct `we.subdiv` constant extraction.
- Commit 5: full `Z` branch decryption for `2018-04-04` and old `KENCRUZR` fixtures.
- Commit 6: `P/P` and `R/R` branch decryption.
- Commit 7: `decrypt_xml` wiring.
- Commit 8: `KrpanoDezoomer` integration and end-to-end validation.
