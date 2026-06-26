use custom_error::custom_error;

pub mod branches;
pub mod codecs;
pub mod crypto;
pub mod header;
pub mod modern_engine;
pub mod old_engine;
pub mod viewer;

#[allow(unused_imports)]
pub use header::{KencBranch, KencHeader};
#[allow(unused_imports)]
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

pub fn decrypt_xml(contents: &[u8], key: Option<&str>) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let payload = encrypted_payload(contents)?;
    let header = KencHeader::parse(&payload)?;
    let _encrypted_body = header.payload(&payload);
    let _key = key.ok_or(EncryptedKrpanoError::MissingKey)?;
    Err(EncryptedKrpanoError::Unsupported)
}

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
            let js_path = viewer_js_path(&dir)
                .unwrap_or_else(|| panic!("missing viewer JS fixture in {}", dir.display()));
            let js = fs::read(&js_path).unwrap();
            let decoded = extract_decoded_viewer_js(&js)
                .unwrap_or_else(|err| panic!("{}: {err}", js_path.display()));
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
    // Phase 1: Fixture metadata tests
    // -----------------------------------------------------------------

    fn fixture_header_info(dir_name: &str) -> (&'static str, KencBranch) {
        match dir_name {
            "old" => ("KENCRUZR", KencBranch::OldZ),
            "2015-08-04" => ("KENCRUZR", KencBranch::OldZ),
            "2017-09-21" => ("KENCRUZR", KencBranch::OldZ),
            "2018-04-04" => ("KENCPUZR", KencBranch::ModernZ),
            "2023-02-07" => ("KENCRURR", KencBranch::RR),
            "2023-04-30" => ("KENCRURR", KencBranch::RR),
            "2023-12-11" => ("KENCRURR", KencBranch::RR),
            "2024-12-20" => ("KENCRURR", KencBranch::RR),
            _ => panic!("unknown fixture directory: {dir_name}"),
        }
    }

    fn fixture_decoded_engine_len(dir_name: &str) -> usize {
        match dir_name {
            "old" => 214_903,
            "2015-08-04" => 191_689,
            "2017-09-21" => 227_010,
            "2018-04-04" => 254_751,
            "2023-02-07" => 359_957,
            "2023-04-30" => 441_405,
            "2023-12-11" => 441_589,
            "2024-12-20" => 482_960,
            _ => panic!("unknown fixture directory: {dir_name}"),
        }
    }

    fn fixture_wrapper_key_len(dir_name: &str) -> usize {
        match dir_name {
            "old" => 136,
            "2015-08-04" => 60,
            "2017-09-21" => 115,
            "2018-04-04" => 17,
            "2023-02-07" => 29,
            "2023-04-30" => 110,
            "2023-12-11" => 163,
            "2024-12-20" => 45,
            _ => panic!("unknown fixture directory: {dir_name}"),
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
            let (expected_header, _expected_branch) = fixture_header_info(dir_name);

            let xml_path = encrypted_xml_path(&dir)
                .unwrap_or_else(|| panic!("missing encrypted XML fixture in {}", dir.display()));
            let xml = fs::read(&xml_path).unwrap();
            let payload = viewer::encrypted_payload(&xml)
                .unwrap_or_else(|err| panic!("{}: {err}", xml_path.display()));
            let header = KencHeader::parse(&payload)
                .unwrap_or_else(|err| panic!("{}: {err}", xml_path.display()));
            assert_eq!(
                header.raw, expected_header,
                "{}: header mismatch",
                xml_path.display()
            );
            checked += 1;
        }
        assert!(checked >= 8, "expected at least 8 fixture directories, found {checked}");
    }

    #[test]
    fn classifies_every_header_branch() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted");
        let mut checked = 0;
        for entry in fs::read_dir(&root).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.is_dir() {
                continue;
            }
            let dir_name = dir.file_name().unwrap().to_str().unwrap();
            let (_expected_header, expected_branch) = fixture_header_info(dir_name);

            let xml_path = encrypted_xml_path(&dir)
                .unwrap_or_else(|| panic!("missing encrypted XML fixture in {}", dir.display()));
            let xml = fs::read(&xml_path).unwrap();
            let payload = viewer::encrypted_payload(&xml)
                .unwrap_or_else(|err| panic!("{}: {err}", xml_path.display()));
            let header = KencHeader::parse(&payload)
                .unwrap_or_else(|err| panic!("{}: {err}", xml_path.display()));
            assert_eq!(
                header.branch(),
                expected_branch,
                "{}: branch mismatch for header {}",
                xml_path.display(),
                header.raw
            );
            checked += 1;
        }
        assert!(checked >= 8, "expected at least 8 fixture directories, found {checked}");
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
            let expected_len = fixture_wrapper_key_len(dir_name);

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
        assert!(checked >= 8, "expected at least 8 fixture directories, found {checked}");
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
            let expected_len = fixture_decoded_engine_len(dir_name);

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
        assert!(checked >= 8, "expected at least 8 fixture directories, found {checked}");
    }

    // -----------------------------------------------------------------
    // Phase 2: Analysis harness
    // -----------------------------------------------------------------

    #[allow(dead_code)]
    struct DecryptStages {
        fixture: String,
        header: KencHeader,
        branch: KencBranch,
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
                "| {fixture:14} | {header:10} | {branch:?} | {key_len:>3} | {engine:>7} | {body:>5} | {b85:>5} | {dec:>5} | {lz4:>6} | {plain:>6} | {prefix}",
                fixture = self.fixture,
                header = self.header.raw,
                branch = self.branch,
                key_len = self
                    .wrapper_key
                    .as_ref()
                    .map_or_else(|| "-".to_string(), |k| k.len().to_string()),
                engine = self.decoded_engine_len,
                body = self.encrypted_body_len,
                b85 = self.body_decoded_len.map_or_else(|| "-".to_string(), |v| v.to_string()),
                dec = self.byte_decrypted_len.map_or_else(|| "-".to_string(), |v| v.to_string()),
                lz4 = self.lz4_decompressed_len.map_or_else(|| "-".to_string(), |v| v.to_string()),
                plain = self.plaintext_len.map_or_else(|| "-".to_string(), |v| v.to_string()),
                prefix = self.plaintext_prefix.as_deref().unwrap_or("-"),
            );
        }
    }

    fn collect_stages(fixture_dir: &Path) -> DecryptStages {
        let dir_name = fixture_dir.file_name().unwrap().to_str().unwrap().to_string();

        let xml_path = encrypted_xml_path(fixture_dir).unwrap_or_else(|| {
            panic!("missing encrypted XML fixture in {}", fixture_dir.display())
        });
        let xml = fs::read(&xml_path).unwrap();
        let payload = viewer::encrypted_payload(&xml).unwrap();
        let header = KencHeader::parse(&payload).unwrap();
        let branch = header.branch();
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
            branch,
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
}
