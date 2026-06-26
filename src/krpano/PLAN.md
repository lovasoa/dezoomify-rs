# Encrypted krpano XML support plan

## Context

`dezoomify-rs` already has a `krpano` dezoomer that can parse normal krpano XML and build tile providers from `<image>`, `<level>`, and shape tags. Some krpano tours, including the HIROX capture that motivated this work, ship an XML file whose top-level content is an `<encrypted>` element instead of readable krpano XML. The existing parser cannot process those files until the encrypted payload has been decrypted and decompressed back into normal XML.

The previous implementation only added detection/extraction scaffolding in `src/krpano/encrypted.rs`. That is useful plumbing, but it is not sufficient: the decryption entrypoint still returns `Unsupported`, so encrypted krpano tours are not actually supported yet.

## What is understood already

- The HIROX sample is krpano, not a new zoomable image format.
- The viewer JavaScript contains an obfuscated loader that first decodes the actual krpano engine source.
- The top-level viewer-loader obfuscation is modified Base85 followed by LZ4 block decompression and `new Function(...)`.
- The actual krpano engine contains a separate file/XML decryption path in its internal resource-loader module.
- In the supplied decrypted krpano source, the relevant XML/file path is centered around:
  - `ra.decryptData(...)`, which decrypts krpano’s encrypted string constants.
  - The internal resource decoding function usually named `w(a, d)` in the minified source.
  - The byte decryption helper usually named `b(a, b)` in the minified source.
- The XML encrypted payloads are wrapped in `<encrypted>` and may be split across multiple CDATA sections, so concatenating CDATA chunks is required before decoding.
- Official `krpanotools encrypt` can produce public-key encrypted files via `-p`; registered/protected viewers can also use private/custom-key encryption via `-key=ID|KEY`.

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

1. Save a tiny readable krpano XML fixture.
2. Use official `krpanotools encrypt -p` to generate a public encrypted fixture from it.
3. Keep the HIROX `tour.xml` capture as a real-world protected/private encrypted fixture.
4. If possible, add one custom-key fixture generated from a licensed/protected test viewer or documented example.
5. Store only small, license-safe fixtures in the repository; keep large or proprietary captures out of tree.

### 2. Port payload extraction and header parsing

1. Keep the current `<encrypted>` detection and split-CDATA concatenation tests.
2. Add a `KencHeader` parser for the leading `KENC....` marker.
3. Add unit tests for known headers from generated public fixtures and the HIROX fixture.
4. Make unsupported/unknown header variants fail with a precise error that includes the header bytes.

### 3. Port the low-level codecs independently

1. Port the modified Base85 decoder used by krpano encrypted payloads.
2. Port the standard LZ4 block decoder used by krpano.
3. Port the Base64 decoder only if the encrypted XML path still needs it after header parsing is understood.
4. Add small unit tests for each codec using vectors extracted from generated fixtures or from the supplied decoded source.

### 4. Port the byte decryption helper

1. Translate the minified helper `b(a, b)` into readable Rust with named variables.
2. Write tests around intermediate buffers from generated public encrypted fixtures.
3. Verify output before LZ4 decompression has the expected krpano size/end-offset header.
4. Repeat with the HIROX payload once key derivation is understood.

### 5. Resolve key and constant derivation

1. Isolate the smallest part of `ra.decryptData(...)` needed to decrypt the constants used by XML/file decryption.
2. Port that string-constant decryptor if the constants cannot be replaced by stable documented values.
3. Extract viewer keys from JavaScript (`h(t, "krp:...")`) and from protected/custom-key metadata where available.
4. Add tests that prove the derived keys decrypt generated fixtures and the HIROX fixture.

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
- Commit 2: fixtures and header parser tests.
- Commit 3: modified Base85 and LZ4 codec ports with tests.
- Commit 4: byte decryption helper port with fixture-driven tests.
- Commit 5: key/constant derivation support.
- Commit 6: `KrpanoDezoomer` integration and end-to-end tests.
