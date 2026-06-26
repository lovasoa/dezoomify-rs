# Encrypted krpano XML support plan

## Context

`dezoomify-rs` already has a `krpano` dezoomer that can parse normal krpano XML and build tile providers from `<image>`, `<level>`, and shape tags. Some krpano tours, including the HIROX capture that motivated this work, ship an XML file whose top-level content is an `<encrypted>` element instead of readable krpano XML. The existing parser cannot process those files until the encrypted payload has been decrypted and decompressed back into normal XML.

Update this file as implementation progresses.

## What is understood already

- The viewer JavaScript contains an obfuscated loader that first decodes the actual krpano engine source.
- The top-level viewer-loader obfuscation is modified Base85 followed by LZ4 block decompression and `new Function(...)`.
- The actual krpano engine contains a separate file/XML decryption path in its internal resource-loader module.
- In the [decrypted krpano source](testdata/krpano/encrypted/2023-04-30/decoded.js), the relevant XML/file path is centered around:
  - `ra.decryptData(...)`, which decrypts krpano’s encrypted string constants.
  - The internal resource decoding function usually named `w(a, d)` in the minified source.
  - The byte decryption helper usually named `b(a, b)` in the minified source.
- The XML encrypted payloads are wrapped in `<encrypted>` and may be split across multiple CDATA sections, so concatenating CDATA chunks is required before decoding.
- Official `krpanotools encrypt` can produce public-key encrypted files via `-p`; registered/protected viewers can also use private/custom-key encryption via `-key=ID|KEY`.
- The packed viewer JavaScript extraction/decompression path is now ported, so all encrypted viewer JS fixtures can be decoded into raw krpano engine source for analysis.

## What is still unclear

- The exact meaning of each `KENC....` header variant seen in encrypted XML payloads, especially how public-key, private-key, compressed, and uncompressed variants map onto the minified engine’s branch conditions.
- How to derive or recover the exact byte keys used by the internal `b(a, b)` helper for:
  - public-key encrypted files,
  - license/protected-viewer encrypted files,
  - custom-key encrypted files.
- Which parts of krpano’s `_()` encrypted string-constant system must be ported to obtain the keys/constants used by the file decryptor.
- Whether every modern krpano version uses the same encrypted XML payload format or whether version-specific decoding needs to be selected from the viewer JavaScript.
- How best to integrate key discovery into the synchronous `Dezoomer` interface without making the generic krpano parser depend on one specific site layout.

## Implementation plan

### 1. Build a focused test corpus

Done, see [testdata](testdata/krpano/encrypted/).
Each folder has a tour.{js,xml} pair that the code in the repo should be able to decode.

### 2. Port payload extraction and header parsing

Status: in progress. `KencHeader` now parses the eight-byte `KENC....` prefix and has tests for the generated public header `KENCPUPR` and the HIROX header `KENCRURR`. The exact semantics of each flag byte are still being investigated.

1. Keep the current `<encrypted>` detection and split-CDATA concatenation tests. ✅
2. Add a `KencHeader` parser for the leading `KENC....` marker. ✅
3. Add unit tests for known headers from generated public fixtures and the HIROX fixture. ✅
4. Make unsupported/unknown header variants fail with a precise error that includes the header bytes. ✅

### 3. Port the low-level codecs independently

Status: in progress. The modified Base85 decoder and standalone LZ4 block decoder are now ported and covered by focused unit tests. They are not wired into `decrypt_xml` until the byte-decryption/key step is implemented.

1. Port the modified Base85 decoder used by krpano encrypted payloads. ✅
2. Port the standard LZ4 block decoder used by krpano. ✅
3. Port the Base64 decoder only if the encrypted XML path still needs it after header parsing is understood. Pending; the current known `KENC...U...` samples use the modified Base85 branch.
4. Add small unit tests for each codec using vectors extracted from generated fixtures or from the supplied decoded source. ✅

### 4. Port the byte decryption helper

Status: in progress. The RC4-like byte decryptor from minified helper `b(a, b)` has been translated to Rust and has a focused synthetic round-trip test. Fixture-driven tests still need real krpano-derived keys/constants.

1. Translate the minified helper `b(a, b)` into readable Rust with named variables. ✅
2. Write tests around intermediate buffers from generated public encrypted fixtures. Pending on key/constant derivation.
3. Verify output before LZ4 decompression has the expected krpano size/end-offset header. Pending on fixture vectors.
4. Repeat with the HIROX payload once key derivation is understood.

### 5. Analyze decoded engine variants and resolve constant derivation

Status: next. The immediate next step is not to guess at constants in Rust, but to analyze every decoded krpano engine variant in the encrypted test corpus and document how each version reaches the XML/file decryptor. That analysis should define the extraction algorithm for the constants and keys needed by XML decoding.

Expected output of this step:

- A version-by-version map of the decoded JS variants under `testdata/krpano/encrypted/`, including the names/structural anchors for:
  - the encrypted string-constant decryptor (`ra.decryptData(...)`, `_()`, or equivalent),
  - the internal file/XML resource decoder (`w(a, d)` or equivalent),
  - the byte decryption helper (`b(a, b)` or equivalent),
  - the `KENC....` header branch logic.
- A list of the exact decrypted constants and viewer keys that feed XML/file decryption.
- A decision on whether one structural extractor can cover the known versions, or whether extraction must be version-specific.
- Fixture-driven test vectors showing the derived constants/keys before they are wired into `decrypt_xml`.

1. Decode or materialize the raw engine source for every encrypted viewer JS fixture, including `2015-06-21`, `2017-02-02`, `2018-12-18`, `2023-02-07`, `2023-04-30`, `2023-12-11`, `2024-12-20`, and `old`.
2. For each decoded engine, locate the string-constant decryptor, XML/file resource decoder, byte decryption helper, and `KENC....` branch logic.
3. Compare those locations across versions and identify stable structural anchors that are safer than minified symbol names.
4. Trace the XML/file decryptor inputs backward to the decrypted constants and viewer-specific keys it uses.
5. Determine which constants are stable/documented and which must be extracted from each viewer JS.
6. Write focused tests that assert the extracted constants/keys for each decoded JS variant.
7. Only after the above map is complete, port the minimal constant/key extraction code needed by XML decoding.

### 6. Integrate into `KrpanoDezoomer`

1. Change `decrypt_xml` from a stub into a real decrypt/decompress function.
2. Extend `KrpanoDezoomer` state only if more than one fetched resource is required, for example XML plus viewer JavaScript.
3. Autodetect encrypted XML and request any missing viewer/key resource via `DezoomerError::NeedsData`.
4. After decryption, feed the plaintext XML into the existing krpano metadata parser unchanged.

### 7. End-to-end validation

1. Add unit tests for each decrypt stage.
2. Add a local dezooming test that proves a small encrypted krpano XML fixture produces the same tile URLs as its plaintext source.
3. Run the existing krpano tests to ensure plaintext support is not regressed.
4. Test the HIROX capture end-to-end, confirming that the generated level and tile URL structure matches the network capture.

## Commit strategy

- Commit 1: this plan document.
- Commit 2: fixtures and header parser tests. In progress: header parsing is implemented; committed fixtures will be added when they are small and license-safe.
- Commit 3: modified Base85 and LZ4 codec ports with tests. In progress: Base85 and LZ4 are ported; Base64 remains pending until a header variant requires it.
- Commit 4: byte decryption helper port with fixture-driven tests. In progress: helper is ported with synthetic coverage; fixture vectors are pending key derivation.
- Commit 5: decoded JS variant analysis, followed by key/constant derivation support once the extraction strategy is clear.
- Commit 6: `KrpanoDezoomer` integration and end-to-end tests.
