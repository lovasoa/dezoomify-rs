use custom_error::custom_error;

pub mod branches;
pub mod codecs;
pub mod crypto;
pub mod header;
pub mod modern_engine;
pub mod old_engine;
pub mod viewer;

pub use header::{BodyCipher, CipherMode, KencHeader};
pub use viewer::{
    encrypted_payload, extract_decoded_viewer_js, extract_key_from_viewer_js, is_encrypted_xml,
};

custom_error! {pub EncryptedKrpanoError
    MissingEncryptedPayload = "encrypted krpano XML did not contain an <encrypted> payload",
    MissingViewerJsPayload = "krpano viewer JavaScript did not contain a decodable embedded payload",
    MissingKey = "encrypted krpano XML needs the krpano viewer JavaScript decryption key",
    MissingKrpKey{candidates: usize, js_len: usize} = "no krp: wrapper key found in viewer JS (scanned {candidates} string literals in {js_len}-byte file; krpano 1.20+ may embed keys differently)",
    HeaderTooShort{len: usize} = "encrypted krpano payload is too short to contain a KENC header (length {len})",
    InvalidHeader{header: String} = "encrypted krpano payload has an invalid KENC header: {header}",
    InvalidBase85Byte{byte: u8} = "encrypted krpano payload contains an invalid modified-base85 byte: {byte}",
    InvalidLz4Block = "encrypted krpano payload contains an invalid LZ4 block",
    InvalidByteCipherInput = "encrypted krpano payload cannot be byte-decrypted with the provided key",
    InvalidUtf8 = "decrypted krpano payload is not valid UTF-8",
    ClassicBAlphabetTooShort{len: usize} = "ClassicB Base64 alphabet has only {len} characters, must be >= 65",
    ClassicBCharNotFound{ch: char, alphabet_len: usize} = "character '{ch}' not found in ClassicB Base64 alphabet ({alphabet_len} chars)",
    Unsupported = "encrypted krpano XML decryption is not implemented for this payload variant yet",
    UnsupportedCombination{cipher: String, mode: String, engine: String} = "KENC combination not supported: cipher={cipher} mode={mode} engine={engine}",
}

// ---------------------------------------------------------------------------
// Engine family detection
// ---------------------------------------------------------------------------

/// Which engine family a decoded viewer JS belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EngineFamily {
    /// Old engine — literal `KENC` in source, numeric `_[]` table,
    /// `b64u8=function` Base64 decoder.
    Old,
    /// Modern engine — no literal `KENC`, uses `we.subdiv` closure.
    Modern,
}

fn detect_engine(decoded_engine: &[u8]) -> EngineFamily {
    let text = match std::str::from_utf8(decoded_engine) {
        Ok(t) => t,
        Err(_) => return EngineFamily::Old,
    };
    // Multiple markers for old engines:
    // - "KENC" literal (most old engines)
    // - "b64u8=function" (old engine Base64 decoder function)
    // - "String(e).charCodeAt" (old engine byte-helper pattern)
    // - "String(h).charCodeAt" (old engine byte-helper pattern variant)
    if text.contains("KENC")
        || text.contains("b64u8=function")
        || text.contains("String(e).charCodeAt")
        || text.contains("String(h).charCodeAt")
    {
        EngineFamily::Old
    } else {
        EngineFamily::Modern
    }
}

// ---------------------------------------------------------------------------
// decrypt_xml — the main entry point
// ---------------------------------------------------------------------------

/// Decrypt an encrypted krpano XML payload.
///
/// `viewer_data` is the raw krpano viewer JavaScript (e.g. `tour.js`).
/// When provided, the wrapper `krp:` key is extracted, the packed engine is
/// decoded, and the header's `BodyCipher` and `CipherMode` determine which
/// key and transform pipeline to use.
///
/// Supported combinations:
///
/// | Cipher     | Mode      | Engine | Header     |
/// |------------|-----------|--------|------------|
/// | ClassicZ   | Public    | modern | `KENCPUZR` |
/// | ClassicZ   | Protected | old    | `KENCRUZR` |
/// | ClassicB   | Public    | old    | `KENCPUBR` |
/// | Subdiv     | Public    | modern | `KENCPUPR` |
/// | Subdiv     | Protected | modern | `KENCRURR` |
pub fn decrypt_xml(
    contents: &[u8],
    viewer_data: Option<&[u8]>,
) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let payload = encrypted_payload(contents)?;
    let header = KencHeader::parse(&payload)?;
    let body = header.payload(&payload);
    log::debug!(
        "decrypt_xml: header={}, cipher={:?}, mode={:?}, body_len={}",
        header.raw,
        header.cipher,
        header.mode,
        body.len()
    );

    let viewer_data = viewer_data.ok_or(EncryptedKrpanoError::MissingKey)?;
    log::debug!("decrypt_xml: viewer_data = {} bytes", viewer_data.len());

    // Extract the wrapper key and decoded engine from the viewer JS.
    let wrapper_key = extract_key_from_viewer_js(viewer_data)?;
    log::debug!("decrypt_xml: wrapper_key length = {}", wrapper_key.len());
    let decoded_engine = extract_decoded_viewer_js(viewer_data)?;
    log::debug!(
        "decrypt_xml: decoded_engine = {} bytes",
        decoded_engine.len()
    );
    let engine = detect_engine(&decoded_engine);
    log::debug!("decrypt_xml: detected engine family = {engine:?}");

    match (header.cipher, header.mode, engine) {
        // ── ClassicZ (Modified Base85 → RC4 → LZ4 → UTF-8) ──
        (BodyCipher::ClassicZ, CipherMode::Public, EngineFamily::Modern) => {
            let ctx = modern_engine::extract_modern_context(&decoded_engine, &wrapper_key)?;
            log::debug!(
                "decrypt_xml: modern ClassicZ, default_key={:?}",
                ctx.default_key
            );
            branches::z_branch_to_plaintext(body, ctx.default_key.as_bytes(), false)
                .map(String::into_bytes)
        }

        (BodyCipher::ClassicZ, CipherMode::Protected, EngineFamily::Old) => {
            let ctx = old_engine::derive_old_license_key(&decoded_engine, &wrapper_key)?;
            let key = ctx.protected_key.ok_or(EncryptedKrpanoError::MissingKey)?;
            log::debug!(
                "decrypt_xml: old ClassicZ, key_variable={}",
                ctx.key_variable
            );
            branches::z_branch_to_plaintext(body, &key, true).map(String::into_bytes)
        }

        // ── ClassicB (Base64 → RC4 → UTF-8) ──
        (BodyCipher::ClassicB, CipherMode::Public, EngineFamily::Old) => {
            let ctx = old_engine::derive_old_license_key(&decoded_engine, &wrapper_key)?;
            let key = &ctx.default_key;
            if key.is_empty() || ctx.base64_alphabet.is_empty() {
                return Err(EncryptedKrpanoError::MissingKey);
            }
            log::debug!("decrypt_xml: old ClassicB, default key");
            branches::b_branch_to_plaintext_with_alphabet(body, &ctx.base64_alphabet, key, false)
                .map(String::into_bytes)
        }

        (BodyCipher::ClassicB, CipherMode::Protected, EngineFamily::Old) => {
            // ClassicB with protected mode — try protected key
            let ctx = old_engine::derive_old_license_key(&decoded_engine, &wrapper_key)?;
            let key = ctx
                .protected_key
                .as_deref()
                .ok_or(EncryptedKrpanoError::MissingKey)?;
            if key.is_empty() || ctx.base64_alphabet.is_empty() {
                return Err(EncryptedKrpanoError::MissingKey);
            }
            log::debug!("decrypt_xml: old ClassicB, protected key");
            branches::b_branch_to_plaintext_with_alphabet(body, &ctx.base64_alphabet, key, true)
                .map(String::into_bytes)
        }

        (BodyCipher::ClassicB, mode, EngineFamily::Modern) => {
            // ClassicB with modern/transitional engine.
            //
            // The Base64 alphabet is normally a custom permutation found in
            // the unpacked rows or the decoded engine source. Engines that
            // embed no custom alphabet (e.g. transitional 1.19-pr16 builds,
            // which have no b64u8 decoder in the source) use standard RFC 4648
            // Base64, so that is the final fallback.
            let ctx = modern_engine::extract_modern_context(&decoded_engine, &wrapper_key)?;
            let alphabet = ctx
                .rows
                .iter()
                .find_map(|row| {
                    let s: String = row
                        .iter()
                        .filter_map(|&c| char::from_u32(u32::from(c)))
                        .collect();
                    if s.len() >= 65 && s.starts_with("ABCDEFGHIJKLMNOPQRSTUVWXYZ") {
                        Some(s)
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    // Fallback: search decoded engine source for alphabet literal
                    std::str::from_utf8(&decoded_engine)
                        .ok()
                        .and_then(old_engine::find_base64_alphabet_in_source)
                })
                .unwrap_or_else(|| {
                    // Final fallback: standard RFC 4648 Base64 (with padding).
                    // Used by transitional engines whose source embeds no
                    // custom alphabet. A wrong alphabet yields non-UTF-8
                    // output that the pipeline rejects, so this is safe.
                    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=".to_string()
                });
            log::debug!(
                "decrypt_xml: modern ClassicB, alphabet={} chars",
                alphabet.len()
            );
            let key = match mode {
                CipherMode::Public => ctx.default_key.as_bytes().to_vec(),
                CipherMode::Protected => {
                    // ClassicB inherits the old-engine `case 7` key derivation:
                    // the `pk=` side-record value's characters (charCodeAt &
                    // 255), padded to 128 by cycling.  The value is NOT
                    // base64-decoded again — `side_records` already decoded the
                    // side-data blob.
                    let records = modern_engine::side_records(&ctx)?;
                    let pk = records
                        .into_iter()
                        .find_map(|record| record.strip_prefix("pk=").map(ToOwned::to_owned))
                        .ok_or(EncryptedKrpanoError::MissingKey)?;
                    old_engine::pad_key_string_to_128(&pk)
                }
            };
            branches::b_branch_to_plaintext_with_alphabet(
                body,
                &alphabet,
                &key,
                mode == CipherMode::Protected,
            )
            .map(String::into_bytes)
        }

        // ── Subdiv (token replacement → we.subdiv branch 5) ──
        (BodyCipher::Subdiv, _, EngineFamily::Modern) => {
            let ctx = modern_engine::extract_modern_context(&decoded_engine, &wrapper_key)?;
            log::debug!(
                "decrypt_xml: modern Subdiv, mode={:?}, checksum={}",
                header.mode,
                ctx.checksum_constant
            );
            modern_engine::pp_rr_branch_to_plaintext(body, &ctx).map(String::into_bytes)
        }

        // ── Unsupported combinations ──
        (BodyCipher::Subdiv, _, EngineFamily::Old) => {
            log::debug!("decrypt_xml: Subdiv cipher with old engine — unsupported");
            Err(EncryptedKrpanoError::Unsupported)
        }

        (cipher, mode, engine) => {
            log::debug!(
                "decrypt_xml: unsupported combination cipher={cipher:?} mode={mode:?} engine={engine:?}"
            );
            Err(EncryptedKrpanoError::UnsupportedCombination {
                cipher: format!("{cipher:?}"),
                mode: format!("{mode:?}"),
                engine: format!("{engine:?}"),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn viewer_js_path(dir: &Path) -> Option<PathBuf> {
        ["tour.js", "krpano.js"]
            .into_iter()
            .map(|name| dir.join(name))
            .find(|path| path.exists())
    }

    fn encrypted_xml_path(dir: &Path) -> Option<PathBuf> {
        ["tour.xml", "krpano.xml"]
            .into_iter()
            .map(|name| dir.join(name))
            .find(|path| path.exists())
    }

    fn looks_like_krpano_xml(text: &str) -> bool {
        let mut text = text.trim_start_matches('\u{feff}').trim_start();
        loop {
            if text.starts_with("<krpano") {
                return true;
            }
            if text.starts_with("<?")
                && let Some(end) = text.find("?>")
            {
                text = text[end + 2..].trim_start();
                continue;
            }
            if text.starts_with("<!--")
                && let Some(end) = text.find("-->")
            {
                text = text[end + 3..].trim_start();
                continue;
            }
            return false;
        }
    }

    /// Compare two byte slices with a readable failure message.
    ///
    /// Unlike `assert_eq!` on `Vec<u8>` (which prints giant arrays of decimal
    /// numbers), this reports lengths, the first differing byte offset, and a
    /// short text context around the divergence.
    fn assert_bytes_eq(actual: &[u8], expected: &[u8], msg: &str) {
        if actual == expected {
            return;
        }
        let common = actual
            .iter()
            .zip(expected.iter())
            .take_while(|(a, b)| a == b)
            .count();
        const CONTEXT: usize = 100;
        let start = common.saturating_sub(CONTEXT);
        let actual_end = (common + CONTEXT).min(actual.len());
        let expected_end = (common + CONTEXT).min(expected.len());
        panic!(
            "{msg}
  actual length:   {}
  expected length: {}
  first difference at byte offset {}
  actual near diff:   {}
  expected near diff: {}",
            actual.len(),
            expected.len(),
            common,
            show_byte_context(&actual[start..actual_end]),
            show_byte_context(&expected[start..expected_end]),
        );
    }

    fn show_byte_context(bytes: &[u8]) -> String {
        match std::str::from_utf8(bytes) {
            Ok(s) => format!("{s:?}"),
            Err(_) => format!("{}", crate::binary_display::display_bytes(bytes)),
        }
    }

    #[test]
    fn decodes_packed_viewer_js_payload() {
        let js = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("testdata/krpano/encrypted/2023-04-30/tour.js"),
        )
        .unwrap();
        let mut expected = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("testdata/krpano/encrypted/2023-04-30/decoded.js"),
        )
        .unwrap();
        if expected.last() == Some(&b'\n') {
            expected.pop();
        }
        let actual = extract_decoded_viewer_js(&js).unwrap();
        assert_bytes_eq(&actual, &expected, "decoded viewer JS does not match expected decoded.js");
    }

    #[test]
    fn decodes_all_encrypted_krpano_viewer_js_fixtures() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted");
        let mut decoded_count = 0;
        for entry in fs::read_dir(&root).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.is_dir() {
                continue;
            }
            let js_path = match viewer_js_path(&dir) {
                Some(p) => p,
                None => continue,
            };
            let js = fs::read(&js_path).unwrap();
            let decoded = match extract_decoded_viewer_js(&js) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let decoded_text = std::str::from_utf8(&decoded).unwrap();
            assert!(
                decoded_text.starts_with("function "),
                "{} decoded to unexpected JavaScript prefix",
                js_path.display()
            );
            assert!(
                decoded_text.contains("loadpano") || decoded_text.contains("embedhtml5"),
                "{} decoded JavaScript did not contain expected krpano viewer markers",
                js_path.display()
            );

            let expected_path = dir.join("decoded.js");
            if expected_path.exists() {
                let mut expected = fs::read(expected_path).unwrap();
                if expected.last() == Some(&b'\n') {
                    expected.pop();
                }
                assert_bytes_eq(
                    &decoded,
                    &expected,
                    &format!("{}: decoded JS does not match expected decoded.js", js_path.display()),
                );
            }
            decoded_count += 1;
        }
        assert!(decoded_count > 0);
    }

    // -----------------------------------------------------------------
    // Fixture metadata
    // -----------------------------------------------------------------

    fn fixture_header_info(dir_name: &str) -> Option<(&'static str, BodyCipher, CipherMode)> {
        match dir_name {
            "old" => Some(("KENCRUZR", BodyCipher::ClassicZ, CipherMode::Protected)),
            "2013-06-05-B" => Some(("KENCPUBR", BodyCipher::ClassicB, CipherMode::Public)),
            "2013-08-09-B" => Some(("KENCPUBR", BodyCipher::ClassicB, CipherMode::Public)),
            "2015-08-04" => Some(("KENCRUZR", BodyCipher::ClassicZ, CipherMode::Protected)),
            "2017-05-10" => Some(("KENCRUZR", BodyCipher::ClassicZ, CipherMode::Protected)),
            "2017-09-21" => Some(("KENCRUZR", BodyCipher::ClassicZ, CipherMode::Protected)),
            "2018-04-04" => Some(("KENCPUZR", BodyCipher::ClassicZ, CipherMode::Public)),
            "2022-01-13" => Some(("KENCPUPR", BodyCipher::Subdiv, CipherMode::Public)),
            "2023-02-07" => Some(("KENCRURR", BodyCipher::Subdiv, CipherMode::Protected)),
            "2023-04-30" => Some(("KENCRURR", BodyCipher::Subdiv, CipherMode::Protected)),
            "2023-04-30-PP" => Some(("KENCPUPR", BodyCipher::Subdiv, CipherMode::Public)),
            "2023-12-11" => Some(("KENCRURR", BodyCipher::Subdiv, CipherMode::Protected)),
            "2024-12-20" => Some(("KENCRURR", BodyCipher::Subdiv, CipherMode::Protected)),
            "2024-12-20-KENCPUZR" => Some(("KENCPUZR", BodyCipher::ClassicZ, CipherMode::Public)),
            "2015-08-04-KENCRUBR" => {
                Some(("KENCRUBR", BodyCipher::ClassicB, CipherMode::Protected))
            }
            "2018-04-23-KENCRUBR" => {
                Some(("KENCRUBR", BodyCipher::ClassicB, CipherMode::Protected))
            }
            "2019-10-15-KENCPUPR-1.20" => {
                Some(("KENCPUPR", BodyCipher::Subdiv, CipherMode::Public))
            }
            "2026-06-25-pp-01_minimal" => {
                Some(("KENCPUPR", BodyCipher::Subdiv, CipherMode::Public))
            }
            "2026-06-25-pp-02_special_chars" => {
                Some(("KENCPUPR", BodyCipher::Subdiv, CipherMode::Public))
            }
            "2026-06-25-pp-03_nested" => Some(("KENCPUPR", BodyCipher::Subdiv, CipherMode::Public)),
            "2026-06-25-pp-04_large" => Some(("KENCPUPR", BodyCipher::Subdiv, CipherMode::Public)),
            "2026-06-25-pp-05_deep" => Some(("KENCPUPR", BodyCipher::Subdiv, CipherMode::Public)),
            "2026-06-25-rr_minimal" => {
                Some(("KENCRURR", BodyCipher::Subdiv, CipherMode::Protected))
            }
            "2026-06-25-rr_tour" => Some(("KENCRURR", BodyCipher::Subdiv, CipherMode::Protected)),
            "2026-06-25-rr_special" => {
                Some(("KENCRURR", BodyCipher::Subdiv, CipherMode::Protected))
            }
            _ => None,
        }
    }

    fn fixture_decoded_engine_len(dir_name: &str) -> Option<usize> {
        match dir_name {
            "old" => Some(214_903),
            "2013-06-05-B" => Some(129_030),
            "2013-08-09-B" => Some(130_544),
            "2015-08-04" => Some(191_689),
            "2017-09-21" => Some(227_010),
            "2018-04-04" => Some(254_751),
            "2023-02-07" => Some(359_957),
            "2023-04-30" => Some(441_405),
            "2023-04-30-PP" => Some(441_405),
            "2023-12-11" => Some(441_589),
            "2024-12-20" => Some(482_960),
            "2024-12-20-KENCPUZR" => Some(482_960),
            "2015-08-04-KENCRUBR" => Some(191_689),
            "2018-04-23-KENCRUBR" => Some(254_755),
            "2019-10-15-KENCPUPR-1.20" => Some(334_009),
            "2026-06-25-pp-01_minimal"
            | "2026-06-25-pp-02_special_chars"
            | "2026-06-25-pp-03_nested"
            | "2026-06-25-pp-04_large"
            | "2026-06-25-pp-05_deep"
            | "2026-06-25-rr_minimal"
            | "2026-06-25-rr_tour"
            | "2026-06-25-rr_special" => Some(550_911),
            _ => None,
        }
    }

    fn fixture_wrapper_key_len(dir_name: &str) -> Option<usize> {
        match dir_name {
            "old" => Some(8778),
            "2013-06-05-B" => Some(6916),
            "2013-08-09-B" => Some(6486),
            "2015-08-04" => Some(7914),
            "2017-09-21" => Some(9412),
            "2018-04-04" => Some(1607),
            "2023-02-07" => Some(2798),
            "2023-04-30" => Some(2915),
            "2023-04-30-PP" => Some(2795),
            "2023-12-11" => Some(2823),
            "2024-12-20" => Some(2874),
            "2024-12-20-KENCPUZR" => Some(2874),
            "2015-08-04-KENCRUBR" => Some(8223),
            "2018-04-23-KENCRUBR" => Some(1768),
            "2019-10-15-KENCPUPR-1.20" => Some(1856),
            // 2019-10-15-KENCPUPR-1.20 uses ptp: wrapper key prefix (krpano 1.20+)
            "2026-06-25-pp-01_minimal"
            | "2026-06-25-pp-02_special_chars"
            | "2026-06-25-pp-03_nested"
            | "2026-06-25-pp-04_large"
            | "2026-06-25-pp-05_deep" => Some(2549),
            "2026-06-25-rr_minimal" => Some(3061),
            "2026-06-25-rr_tour" => Some(3053),
            "2026-06-25-rr_special" => Some(3055),
            _ => None,
        }
    }

    #[test]
    fn all_fixtures_have_correct_kenc_header() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted");
        let mut checked = 0;
        for entry in fs::read_dir(&root).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.is_dir() {
                continue;
            }
            let dir_name = dir.file_name().unwrap().to_str().unwrap();
            let (expected_header, _expected_cipher, _expected_mode) =
                match fixture_header_info(dir_name) {
                    Some(v) => v,
                    None => continue,
                };

            let xml_path = encrypted_xml_path(&dir)
                .unwrap_or_else(|| panic!("missing encrypted XML fixture in {}", dir.display()));
            let xml = fs::read(&xml_path).unwrap();
            let payload = viewer::encrypted_payload(&xml)
                .unwrap_or_else(|err| panic!("{}: {err}", xml_path.display()));
            let header = KencHeader::parse(&payload)
                .unwrap_or_else(|err| panic!("{}: {err}", xml_path.display()));
            assert_eq!(
                header.raw,
                expected_header,
                "{}: header mismatch",
                xml_path.display()
            );
            checked += 1;
        }
        assert!(
            checked >= 19,
            "expected at least 19 fixture directories, found {checked}"
        );
    }

    #[test]
    fn all_fixtures_classify_correctly() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted");
        let mut checked = 0;
        for entry in fs::read_dir(&root).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.is_dir() {
                continue;
            }
            let dir_name = dir.file_name().unwrap().to_str().unwrap();
            let (_expected_header, expected_cipher, expected_mode) =
                match fixture_header_info(dir_name) {
                    Some(v) => v,
                    None => continue,
                };

            let xml_path = encrypted_xml_path(&dir)
                .unwrap_or_else(|| panic!("missing encrypted XML fixture in {}", dir.display()));
            let xml = fs::read(&xml_path).unwrap();
            let payload = viewer::encrypted_payload(&xml)
                .unwrap_or_else(|err| panic!("{}: {err}", xml_path.display()));
            let header = KencHeader::parse(&payload)
                .unwrap_or_else(|err| panic!("{}: {err}", xml_path.display()));
            assert_eq!(
                header.cipher,
                expected_cipher,
                "{}: cipher mismatch for header {}",
                xml_path.display(),
                header.raw
            );
            assert_eq!(
                header.mode,
                expected_mode,
                "{}: mode mismatch for header {}",
                xml_path.display(),
                header.raw
            );
            checked += 1;
        }
        assert!(
            checked >= 19,
            "expected at least 19 fixture directories, found {checked}"
        );
    }

    #[test]
    fn all_fixtures_extract_correct_wrapper_key_length() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted");
        let mut checked = 0;
        for entry in fs::read_dir(&root).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.is_dir() {
                continue;
            }
            let dir_name = dir.file_name().unwrap().to_str().unwrap();
            let expected_len = match fixture_wrapper_key_len(dir_name) {
                Some(v) => v,
                None => continue,
            };

            let js_path = viewer_js_path(&dir)
                .unwrap_or_else(|| panic!("missing viewer JS fixture in {}", dir.display()));
            let js = fs::read(&js_path).unwrap();
            let key = extract_key_from_viewer_js(&js)
                .unwrap_or_else(|_| panic!("{}: no krp: key found", js_path.display()));
            assert_eq!(
                key.len(),
                expected_len,
                "{}: wrapper key length mismatch",
                js_path.display()
            );
            checked += 1;
        }
        assert!(
            checked >= 19,
            "expected at least 19 fixture directories, found {checked}"
        );
    }

    #[test]
    fn all_fixtures_decode_viewer_js_to_expected_length() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted");
        let mut checked = 0;
        for entry in fs::read_dir(&root).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.is_dir() {
                continue;
            }
            let dir_name = dir.file_name().unwrap().to_str().unwrap();
            let expected_len = match fixture_decoded_engine_len(dir_name) {
                Some(v) => v,
                None => continue,
            };

            let js_path = viewer_js_path(&dir)
                .unwrap_or_else(|| panic!("missing viewer JS fixture in {}", dir.display()));
            let js = fs::read(&js_path).unwrap();
            let decoded = extract_decoded_viewer_js(&js)
                .unwrap_or_else(|err| panic!("{}: {err}", js_path.display()));
            assert_eq!(
                decoded.len(),
                expected_len,
                "{}: decoded engine length mismatch",
                js_path.display()
            );
            checked += 1;
        }
        assert!(
            checked >= 11,
            "expected at least 11 fixture directories, found {checked}"
        );
    }

    // -----------------------------------------------------------------
    // Analysis harness
    // -----------------------------------------------------------------

    #[allow(dead_code)]
    struct DecryptStages {
        fixture: String,
        header: KencHeader,
        cipher: BodyCipher,
        mode: CipherMode,
        wrapper_key: Option<String>,
        decoded_engine_len: usize,
        encrypted_body_len: usize,
        body_decoded_len: Option<usize>,
        byte_decrypted_len: Option<usize>,
        lz4_decompressed_len: Option<usize>,
        plaintext_len: Option<usize>,
        plaintext_prefix: Option<String>,
    }

    impl DecryptStages {
        fn print_row(&self) {
            eprintln!(
                "| {fixture:14} | {header:10} | {cipher:?} | {mode:?} | {key_len:>3} | {engine:>7} | {body:>5} | {b85:>5} | {dec:>5} | {lz4:>6} | {plain:>6} | {prefix}",
                fixture = self.fixture,
                header = self.header.raw,
                cipher = self.cipher,
                mode = self.mode,
                key_len = self
                    .wrapper_key
                    .as_ref()
                    .map_or_else(|| "-".to_string(), |k| k.len().to_string()),
                engine = self.decoded_engine_len,
                body = self.encrypted_body_len,
                b85 = self
                    .body_decoded_len
                    .map_or_else(|| "-".to_string(), |v| v.to_string()),
                dec = self
                    .byte_decrypted_len
                    .map_or_else(|| "-".to_string(), |v| v.to_string()),
                lz4 = self
                    .lz4_decompressed_len
                    .map_or_else(|| "-".to_string(), |v| v.to_string()),
                plain = self
                    .plaintext_len
                    .map_or_else(|| "-".to_string(), |v| v.to_string()),
                prefix = self.plaintext_prefix.as_deref().unwrap_or("-"),
            );
        }
    }

    fn collect_stages(fixture_dir: &Path) -> DecryptStages {
        let dir_name = fixture_dir
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let xml_path = encrypted_xml_path(fixture_dir).unwrap_or_else(|| {
            panic!("missing encrypted XML fixture in {}", fixture_dir.display())
        });
        let xml = fs::read(&xml_path).unwrap();
        let payload = viewer::encrypted_payload(&xml).unwrap();
        let header = KencHeader::parse(&payload).unwrap();
        let cipher = header.cipher;
        let mode = header.mode;
        let body = header.payload(&payload);
        let encrypted_body_len = body.len();

        let js_path = viewer_js_path(fixture_dir);
        let wrapper_key = js_path.as_ref().and_then(|p| {
            let js = fs::read(p).ok()?;
            extract_key_from_viewer_js(&js).ok()
        });
        let decoded_engine_len = js_path
            .as_ref()
            .and_then(|p| fs::read(p).ok())
            .and_then(|js| extract_decoded_viewer_js(&js).ok())
            .map_or(0, |d| d.len());

        DecryptStages {
            fixture: dir_name,
            header,
            cipher,
            mode,
            wrapper_key,
            decoded_engine_len,
            encrypted_body_len,
            body_decoded_len: None,
            byte_decrypted_len: None,
            lz4_decompressed_len: None,
            plaintext_len: None,
            plaintext_prefix: None,
        }
    }

    #[test]
    #[ignore]
    fn analysis_harness_prints_all_stages() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted");
        let mut stages: Vec<DecryptStages> = Vec::new();
        for entry in fs::read_dir(&root).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.is_dir() {
                continue;
            }
            stages.push(collect_stages(&dir));
        }
        stages.sort_by(|a, b| a.fixture.cmp(&b.fixture));
        for s in &stages {
            s.print_row();
        }
        assert!(!stages.is_empty());
    }

    // -----------------------------------------------------------------
    // End-to-end decryption tests
    // -----------------------------------------------------------------

    #[test]
    fn decrypt_xml_2018_04_04() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted/2018-04-04");
        let xml = fs::read(root.join("tour.xml")).unwrap();
        let js = fs::read(root.join("tour.js")).unwrap();

        let plaintext = decrypt_xml(&xml, Some(&js)).unwrap();
        assert_eq!(plaintext.len(), 36407, "plaintext length");
        let text = std::str::from_utf8(&plaintext).unwrap();
        assert!(
            looks_like_krpano_xml(text),
            "plaintext should start with <krpano>"
        );
    }

    #[test]
    fn decrypt_xml_protected_subdiv_fixtures() {
        for fixture in ["2023-02-07", "2023-04-30", "2023-12-11", "2024-12-20"] {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("testdata/krpano/encrypted")
                .join(fixture);
            let xml_path = encrypted_xml_path(&root)
                .unwrap_or_else(|| panic!("{fixture}: missing encrypted XML"));
            let js_path =
                viewer_js_path(&root).unwrap_or_else(|| panic!("{fixture}: missing viewer JS"));
            let xml = fs::read(xml_path).unwrap();
            let js = fs::read(js_path).unwrap();

            let plaintext =
                decrypt_xml(&xml, Some(&js)).unwrap_or_else(|err| panic!("{fixture}: {err}"));
            let text = std::str::from_utf8(&plaintext)
                .unwrap_or_else(|err| panic!("{fixture}: plaintext is not UTF-8: {err}"));
            let normalized = text.trim_start_matches('\u{feff}').trim_start();
            assert!(
                looks_like_krpano_xml(text),
                "{fixture}: plaintext should start with <krpano>, got prefix: {:?}",
                &normalized[..normalized.len().min(200)]
            );
            let _parsed: PlaintextKrpanoRoot = serde_xml_rs::from_reader(text.as_bytes())
                .unwrap_or_else(|err| panic!("{fixture}: plaintext XML did not parse: {err}"));
        }
    }

    #[test]
    fn decrypt_xml_public_subdiv_fixture() {
        let fixture = "2023-04-30-PP";
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/krpano/encrypted")
            .join(fixture);
        let xml_path =
            encrypted_xml_path(&root).unwrap_or_else(|| panic!("{fixture}: missing encrypted XML"));
        let js_path =
            viewer_js_path(&root).unwrap_or_else(|| panic!("{fixture}: missing viewer JS"));
        let xml = fs::read(xml_path).unwrap();
        let js = fs::read(js_path).unwrap();

        let plaintext =
            decrypt_xml(&xml, Some(&js)).unwrap_or_else(|err| panic!("{fixture}: {err}"));
        let text = std::str::from_utf8(&plaintext)
            .unwrap_or_else(|err| panic!("{fixture}: plaintext is not UTF-8: {err}"));
        assert!(
            looks_like_krpano_xml(text),
            "{fixture}: plaintext should start with <krpano>"
        );
        let _parsed: PlaintextKrpanoRoot = serde_xml_rs::from_reader(text.as_bytes())
            .unwrap_or_else(|err| panic!("{fixture}: plaintext XML did not parse: {err}"));
    }

    #[test]
    fn decrypt_xml_transitional_classicb_fixture() {
        // 2018-04-23-KENCRUBR: krpano 1.19-pr16 transitional engine.
        // ClassicB + Protected with a standard Base64 alphabet and a pk=-derived
        // case-7 key. Exercises the modern-ClassicB dispatch arm.
        let fixture = "2018-04-23-KENCRUBR";
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/krpano/encrypted")
            .join(fixture);
        let xml_path =
            encrypted_xml_path(&root).unwrap_or_else(|| panic!("{fixture}: missing encrypted XML"));
        let js_path =
            viewer_js_path(&root).unwrap_or_else(|| panic!("{fixture}: missing viewer JS"));
        let xml = fs::read(xml_path).unwrap();
        let js = fs::read(js_path).unwrap();

        let plaintext =
            decrypt_xml(&xml, Some(&js)).unwrap_or_else(|err| panic!("{fixture}: {err}"));
        let text = std::str::from_utf8(&plaintext)
            .unwrap_or_else(|err| panic!("{fixture}: plaintext is not UTF-8: {err}"));
        assert!(
            looks_like_krpano_xml(text),
            "{fixture}: plaintext should start with <krpano>"
        );
        let _parsed: PlaintextKrpanoRoot = serde_xml_rs::from_reader(text.as_bytes())
            .unwrap_or_else(|err| panic!("{fixture}: plaintext XML did not parse: {err}"));
    }

    #[test]
    fn decrypt_xml_old_fixtures() {
        for fixture in [
            "old",
            "2013-06-05-B",
            "2013-08-09-B",
            "2015-08-04",
            "2015-08-04-KENCRUBR",
            "2017-09-21",
        ] {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("testdata/krpano/encrypted")
                .join(fixture);
            let xml_path = encrypted_xml_path(&root)
                .unwrap_or_else(|| panic!("{fixture}: missing encrypted XML"));
            let js_path =
                viewer_js_path(&root).unwrap_or_else(|| panic!("{fixture}: missing viewer JS"));
            let xml = fs::read(xml_path).unwrap();
            let js = fs::read(js_path).unwrap();

            let plaintext =
                decrypt_xml(&xml, Some(&js)).unwrap_or_else(|err| panic!("{fixture}: {err}"));
            let text = std::str::from_utf8(&plaintext)
                .unwrap_or_else(|err| panic!("{fixture}: plaintext is not UTF-8: {err}"));
            assert!(
                looks_like_krpano_xml(text),
                "{fixture}: plaintext should start with <krpano>"
            );
            let _parsed: PlaintextKrpanoRoot = serde_xml_rs::from_reader(text.as_bytes())
                .unwrap_or_else(|err| panic!("{fixture}: plaintext XML did not parse: {err}"));
        }
    }

    /// End-to-end: iterate every encrypted krpano fixture subfolder, decrypt,
    /// and assert the result is valid XML. When a `plaintext.xml` is present,
    /// also assert exact byte-for-byte match.
    ///
    /// Every fixture that has both an encrypted XML and a viewer JS is tested.
    /// Failures are collected and reported together at the end.
    #[test]
    fn decrypt_xml_all_fixtures_to_valid_xml() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted");
        let mut tested = 0;
        let mut failures: Vec<String> = Vec::new();
        for entry in fs::read_dir(&root).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.is_dir() {
                continue;
            }
            let dir_name = dir.file_name().unwrap().to_str().unwrap();

            let xml_path = match encrypted_xml_path(&dir) {
                Some(p) => p,
                None => continue,
            };
            let js_path = match viewer_js_path(&dir) {
                Some(p) => p,
                None => continue,
            };

            let xml = fs::read(&xml_path).unwrap();
            let js = fs::read(&js_path).unwrap();

            match decrypt_xml(&xml, Some(&js)) {
                Ok(plaintext) => {
                    let text = std::str::from_utf8(&plaintext)
                        .unwrap_or_else(|err| panic!("{dir_name}: plaintext is not UTF-8: {err}"));
                    let normalized = text.trim_start_matches('\u{feff}').trim_start();
                    assert!(
                        looks_like_krpano_xml(text),
                        "{dir_name}: plaintext should start with <krpano>, got prefix: {:?}",
                        &normalized[..normalized.len().min(200)]
                    );
                    let _parsed: PlaintextKrpanoRoot = serde_xml_rs::from_reader(text.as_bytes())
                        .unwrap_or_else(|err| {
                            panic!("{dir_name}: plaintext XML did not parse: {err}")
                        });

                    let expected_path = dir.join("plaintext.xml");
                    if expected_path.exists() {
                        let mut expected = fs::read(&expected_path).unwrap();
                        if expected.last() == Some(&b'\n') {
                            expected.pop();
                        }
                        let mut actual = plaintext.clone();
                        if actual.last() == Some(&b'\n') {
                            actual.pop();
                        }
                        assert_bytes_eq(
                            &actual,
                            &expected,
                            &format!("{dir_name}: plaintext does not match expected plaintext.xml"),
                        );
                    }
                    tested += 1;
                }
                Err(err) => {
                    failures.push(format!("{dir_name}: {err}"));
                }
            }
        }

        if !failures.is_empty() {
            panic!(
                "{} fixture(s) failed to decrypt:\n\n{}\n",
                failures.len(),
                failures.join("\n")
            );
        }

        assert!(
            tested >= 18,
            "expected at least 18 successful fixture directories, found {tested}"
        );
    }

    #[test]
    fn probe_external_repos() {
        use std::collections::BTreeMap;
        use std::fs;
        let candidates: &[(&str, &str, &str)] = &[
            // KENCPUBR - ClassicB+Public+Old
            (
                "/tmp/kenc-repos/SenYuanZ__Museum-News/News-2/bwg3d/plugins/map_core.xml",
                "/tmp/kenc-repos/SenYuanZ__Museum-News/News-2/bwg3d/plugins/map_core.js",
                "KENCPUBR: map_core",
            ),
            // KENCPUPR - Subdiv+Public+Modern
            (
                "/tmp/kenc-repos/SanyoRadio__Saronida-Panorama/saro.xml",
                "/tmp/kenc-repos/SanyoRadio__Saronida-Panorama/saro.js",
                "KENCPUPR: saro 1.20.2",
            ),
            // KENCPUZR - ClassicZ+Public+Modern
            (
                "/tmp/kenc-repos/iflycn__vr/inc/pano_webvr.xml",
                "/tmp/kenc-repos/iflycn__vr/inc/pano_webvr.js",
                "KENCPUZR: pano_webvr",
            ),
            // KENCRURR - Subdiv+Protected+Modern
            (
                "/tmp/kenc-repos/poricf__Vr-secondround/Lycee(Newroad)_Type A3/tour.xml",
                "/tmp/kenc-repos/poricf__Vr-secondround/Lycee(Newroad)_Type A3/tour.js",
                "KENCRURR: Lycee 1.21",
            ),
            (
                "/tmp/kenc-repos/parakhc4__Vincent_inn_3D_Tour/tour.xml",
                "/tmp/kenc-repos/parakhc4__Vincent_inn_3D_Tour/tour.js",
                "KENCRURR: Vincent 1.21",
            ),
            // More KENCRURR pairs
            (
                "/tmp/kenc-repos/Dilhakk__Temervr/Seken_Lycee(Type_08)/tour.xml",
                "/tmp/kenc-repos/Dilhakk__Temervr/Seken_Lycee(Type_08)/tour.js",
                "KENCRURR: Seken Lycee",
            ),
            // KENCRUZR - ClassicZ+Protected+Old
            (
                "/tmp/kenc-repos/poricf__Vr-secondround/Lycee(Newroad)_Type A3/plugins/webvr.xml",
                "/tmp/kenc-repos/poricf__Vr-secondround/Lycee(Newroad)_Type A3/plugins/webvr.js",
                "KENCRUZR: webvr plugin",
            ),
            (
                "/tmp/kenc-repos/duheng__vrseat/src/setting/tour.xml",
                "/tmp/kenc-repos/duheng__vrseat/src/setting/tour.js",
                "KENCRUZR: duheng 1.19",
            ),
            // More KENCRUZR pairs
            (
                "/tmp/kenc-repos/tinyhousecn__Toilet-Expandable-Container-House-Kaipu/pano.xml",
                "/tmp/kenc-repos/tinyhousecn__Toilet-Expandable-Container-House-Kaipu/pano.js",
                "KENCRUZR: tinyhousecn",
            ),
            // KENCRUBR - ClassicB+Protected
            (
                "/tmp/kenc-repos/iamsayan__virtual-tours/2022/e55b32193661624c300000df/32data/32.xml",
                "/tmp/kenc-repos/iamsayan__virtual-tours/2022/e55b32193661624c300000df/32data/32.js",
                "KENCRUBR: iamsayan 32",
            ),
            (
                "/tmp/kenc-repos/iNATS__inatsVr/iNATS Demodata/iNATS Demo.xml",
                "/tmp/kenc-repos/iNATS__inatsVr/iNATS Demodata/iNATS Demo.js",
                "KENCRUBR: iNATS",
            ),
            (
                "/tmp/kenc-repos/Azat301__my/1.xml",
                "/tmp/kenc-repos/Azat301__my/1.js",
                "KENCRUBR: Azat301 1",
            ),
        ];

        let mut summaries: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for (xml_path, js_path, label) in candidates {
            let xml = match fs::read(xml_path) {
                Ok(x) => x,
                Err(_) => {
                    summaries
                        .entry("XML_NOT_FOUND".into())
                        .or_default()
                        .push(label.to_string());
                    continue;
                }
            };
            let js = match fs::read(js_path) {
                Ok(x) => x,
                Err(_) => {
                    summaries
                        .entry("JS_NOT_FOUND".into())
                        .or_default()
                        .push(label.to_string());
                    continue;
                }
            };
            let Ok(payload) = viewer::encrypted_payload(&xml) else {
                summaries
                    .entry("FAIL: payload_extraction".into())
                    .or_default()
                    .push(label.to_string());
                continue;
            };
            let _header = match KencHeader::parse(&payload) {
                Ok(h) => h,
                Err(e) => {
                    summaries
                        .entry(format!("FAIL: header_parse({e})"))
                        .or_default()
                        .push(label.to_string());
                    continue;
                }
            };
            let wrapper_key = extract_key_from_viewer_js(&js);
            let decoded_engine = match extract_decoded_viewer_js(&js) {
                Ok(d) => d,
                Err(e) => {
                    summaries
                        .entry(format!("FAIL: decode_viewer_js({e})"))
                        .or_default()
                        .push(label.to_string());
                    continue;
                }
            };
            let engine = detect_engine(&decoded_engine);
            match decrypt_xml(&xml, Some(&js)) {
                Ok(plaintext) => {
                    summaries.entry("OK".into()).or_default().push(format!(
                        "{label} plain={} engine={engine:?}",
                        plaintext.len()
                    ));
                }
                Err(e) => {
                    let key = format!(
                        "FAIL: decrypt engine={engine:?} wk={} de={} err={e}",
                        wrapper_key.as_ref().map_or(0, |k: &String| k.len()),
                        decoded_engine.len()
                    );
                    summaries.entry(key).or_default().push(label.to_string());
                }
            }
        }

        eprintln!("\n=== RESULTS BY CATEGORY ===");
        for (category, labels) in &summaries {
            eprintln!("  [{category}]");
            for l in labels {
                eprintln!("    {l}");
            }
        }
        eprintln!("\n{} categories total", summaries.len());
    }

    #[test]
    fn probe_external_pair_from_env() {
        let xml_path = match std::env::var_os("KRPANO_PROBE_XML") {
            Some(path) => PathBuf::from(path),
            None => return,
        };
        let js_path = PathBuf::from(
            std::env::var_os("KRPANO_PROBE_JS").expect("KRPANO_PROBE_JS must be set"),
        );
        let xml = fs::read(&xml_path).unwrap();
        let js = fs::read(&js_path).unwrap();
        let payload = viewer::encrypted_payload(&xml).unwrap();
        let header = KencHeader::parse(&payload).unwrap();
        let wrapper_key = extract_key_from_viewer_js(&js)
            .unwrap_or_else(|_| panic!("{}: no krp: key found", js_path.display()));
        let decoded_engine = extract_decoded_viewer_js(&js)
            .unwrap_or_else(|err| panic!("{}: {err}", js_path.display()));

        let plaintext = decrypt_xml(&xml, Some(&js))
            .unwrap_or_else(|err| panic!("{} + {}: {err}", xml_path.display(), js_path.display()));
        let text = std::str::from_utf8(&plaintext)
            .unwrap_or_else(|err| panic!("plaintext is not UTF-8: {err}"));
        assert!(
            looks_like_krpano_xml(text),
            "plaintext should start with <krpano>, got prefix: {:?}",
            &text[..text.len().min(200)]
        );
        let _parsed: PlaintextKrpanoRoot = serde_xml_rs::from_reader(text.as_bytes())
            .unwrap_or_else(|err| panic!("plaintext XML did not parse: {err}"));
        println!(
            "probe ok header={} wrapper_len={} decoded_engine_len={} plaintext_len={} xml={} js={}",
            header.raw,
            wrapper_key.len(),
            decoded_engine.len(),
            plaintext.len(),
            xml_path.display(),
            js_path.display()
        );
    }

    #[derive(serde::Deserialize)]
    #[serde(rename = "krpano")]
    struct PlaintextKrpanoRoot {}
}
