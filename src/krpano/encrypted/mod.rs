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
    HeaderTooShort{len: usize} = "encrypted krpano payload is too short to contain a KENC header (length {len})",
    InvalidHeader{header: String} = "encrypted krpano payload has an invalid KENC header: {header}",
    InvalidBase85Byte{byte: u8} = "encrypted krpano payload contains an invalid modified-base85 byte: {byte}",
    InvalidLz4Block = "encrypted krpano payload contains an invalid LZ4 block",
    InvalidByteCipherInput = "encrypted krpano payload cannot be byte-decrypted with the provided key",
    Unsupported = "encrypted krpano XML decryption is not implemented for this payload variant yet",
}

// ---------------------------------------------------------------------------
// Engine family detection
// ---------------------------------------------------------------------------

/// Which engine family a decoded viewer JS belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EngineFamily {
    /// Old engine — literal `KENC` in source, numeric `_[]` table.
    Old,
    /// Modern engine — no literal `KENC`, uses `we.subdiv` closure.
    Modern,
}

fn detect_engine(decoded_engine: &[u8]) -> EngineFamily {
    let text = match std::str::from_utf8(decoded_engine) {
        Ok(t) => t,
        Err(_) => return EngineFamily::Old,
    };
    if text.contains("KENC") {
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
    let wrapper_key =
        extract_key_from_viewer_js(viewer_data).ok_or(EncryptedKrpanoError::MissingKey)?;
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
        (BodyCipher::ClassicB, CipherMode::Protected, EngineFamily::Old) => {
            // ClassicB with protected mode — not observed, but try protected key
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

        (cipher, mode, engine) => {
            log::debug!(
                "decrypt_xml: unsupported combination cipher={cipher:?} mode={mode:?} engine={engine:?}"
            );
            Err(EncryptedKrpanoError::Unsupported)
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
        assert_eq!(extract_decoded_viewer_js(&js).unwrap(), expected);
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
                assert_eq!(decoded, expected);
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
            "2017-09-21" => Some(("KENCRUZR", BodyCipher::ClassicZ, CipherMode::Protected)),
            "2018-04-04" => Some(("KENCPUZR", BodyCipher::ClassicZ, CipherMode::Public)),
            "2023-02-07" => Some(("KENCRURR", BodyCipher::Subdiv, CipherMode::Protected)),
            "2023-04-30" => Some(("KENCRURR", BodyCipher::Subdiv, CipherMode::Protected)),
            "2023-04-30-PP" => Some(("KENCPUPR", BodyCipher::Subdiv, CipherMode::Public)),
            "2023-12-11" => Some(("KENCRURR", BodyCipher::Subdiv, CipherMode::Protected)),
            "2024-12-20" => Some(("KENCRURR", BodyCipher::Subdiv, CipherMode::Protected)),
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
                .unwrap_or_else(|| panic!("{}: no krp: key found", js_path.display()));
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
            extract_key_from_viewer_js(&js)
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
            text.trim().starts_with("<krpano"),
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
                normalized.starts_with("<krpano"),
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
        let normalized = text.trim_start_matches('\u{feff}').trim_start();
        assert!(
            normalized.starts_with("<krpano"),
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
            let normalized = text.trim_start_matches('\u{feff}').trim_start();
            assert!(
                normalized.starts_with("<krpano"),
                "{fixture}: plaintext should start with <krpano>"
            );
            let _parsed: PlaintextKrpanoRoot = serde_xml_rs::from_reader(text.as_bytes())
                .unwrap_or_else(|err| panic!("{fixture}: plaintext XML did not parse: {err}"));
        }
    }

    #[derive(serde::Deserialize)]
    #[serde(rename = "krpano")]
    struct PlaintextKrpanoRoot {}
}
