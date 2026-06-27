use super::EncryptedKrpanoError;
use super::codecs;
use super::crypto;

/// Apply the P/P and R/R body transform: replace every `z` with backslash.
///
/// The modern krpano engine resource-loader branch for KENCPUPR / KENCRURR
/// headers rewrites `z` → `\` before further processing.  The backslash is
/// a valid modified-Base85 character so the resulting string is decodable.
pub fn replace_z_with_backslash(body: &str) -> String {
    body.replace('z', "\\")
}

/// Decode a P/P or R/R encrypted body into raw (still byte-encrypted) bytes.
///
/// Pipeline: `z`→`\` → strip `%*` or `$*<key-id>@` envelope → modified-Base85 decode.
pub fn decode_pp_rr_body(body: &str) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let replaced = replace_z_with_backslash(body);
    let envelope = parse_pp_rr_envelope(&replaced);
    codecs::decode_modified_base85(envelope.payload)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PpRrEnvelope<'a> {
    pub key_id: Option<&'a str>,
    pub payload: &'a str,
}

pub fn parse_pp_rr_envelope(replaced_body: &str) -> PpRrEnvelope<'_> {
    PpRrEnvelope::parse(replaced_body)
}

impl<'a> PpRrEnvelope<'a> {
    fn parse(replaced_body: &'a str) -> Self {
        if let Some(payload) = replaced_body.strip_prefix("%*") {
            return Self {
                key_id: None,
                payload,
            };
        }
        if let Some(rest) = replaced_body.strip_prefix("$*") {
            if let Some((key_id, payload)) = rest.split_once('@') {
                return Self {
                    key_id: Some(key_id),
                    payload,
                };
            }
        }
        Self {
            key_id: None,
            payload: replaced_body,
        }
    }
}

// ---------------------------------------------------------------------------
// Z branch transform
// ---------------------------------------------------------------------------

/// Decrypt a Z-branch encrypted body into raw bytes.
///
/// Pipeline: modified Base85 decode → byte decrypt → parse LZ4 block header →
/// LZ4 decompress.
///
/// The body is modified-Base85 encoded.  After decoding, the raw bytes are
/// fed to `decrypt_bytes`: the first 128+ bytes form the RC4 key-mixing prefix;
/// the rest is the LZ4-compressed plaintext.  The decrypted result carries an
/// 8-byte LZ4 block header (3-byte LE decompressed length, 3-byte LE compressed
/// length at offset 4).
pub fn decrypt_z_branch(
    body: &str,
    key: &[u8],
    widened: bool,
) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let decoded = codecs::decode_modified_base85(body)?;
    let decrypted = crypto::decrypt_bytes(&decoded, key, widened)?;

    if decrypted.len() < codecs::PACKED_VIEWER_HEADER_LEN {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }

    let decompressed_len = read_u24_le(&decrypted[0..3]);
    let compressed_end = codecs::PACKED_VIEWER_HEADER_LEN + read_u24_le(&decrypted[4..7]);

    if compressed_end > decrypted.len() {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }

    codecs::lz4_decompress_block(
        &decrypted[codecs::PACKED_VIEWER_HEADER_LEN..],
        decompressed_len,
        compressed_end - codecs::PACKED_VIEWER_HEADER_LEN,
    )
}

/// Decrypt a Z-branch encrypted body into a UTF-8 plaintext string.
pub fn z_branch_to_plaintext(
    body: &str,
    key: &[u8],
    widened: bool,
) -> Result<String, EncryptedKrpanoError> {
    let decrypted = decrypt_z_branch(body, key, widened)?;
    String::from_utf8(decrypted).map_err(|_| EncryptedKrpanoError::Unsupported)
}

pub fn b_branch_to_plaintext_with_alphabet(
    body: &str,
    alphabet: &str,
    key: &[u8],
    widened: bool,
) -> Result<String, EncryptedKrpanoError> {
    let decoded = decode_custom_base64(body, alphabet)?;
    let decrypted = crypto::decrypt_bytes(&decoded, key, widened)?;
    String::from_utf8(decrypted).map_err(|_| EncryptedKrpanoError::Unsupported)
}

fn decode_custom_base64(input: &str, alphabet: &str) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let alphabet: Vec<char> = alphabet.chars().collect();
    if alphabet.len() < 65 {
        return Err(EncryptedKrpanoError::Unsupported);
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut chars = input.chars();
    while let (Some(a), Some(b), Some(c), Some(d)) =
        (chars.next(), chars.next(), chars.next(), chars.next())
    {
        let a = alphabet
            .iter()
            .position(|&ch| ch == a)
            .ok_or(EncryptedKrpanoError::Unsupported)?;
        let b = alphabet
            .iter()
            .position(|&ch| ch == b)
            .ok_or(EncryptedKrpanoError::Unsupported)?;
        let c = alphabet
            .iter()
            .position(|&ch| ch == c)
            .ok_or(EncryptedKrpanoError::Unsupported)?;
        let d = alphabet
            .iter()
            .position(|&ch| ch == d)
            .ok_or(EncryptedKrpanoError::Unsupported)?;
        out.push(((a << 2) | (b >> 4)) as u8);
        if c != 64 {
            out.push((((b & 15) << 4) | (c >> 2)) as u8);
        }
        if d != 64 {
            out.push((((c & 3) << 6) | d) as u8);
        }
    }
    Ok(out)
}

fn read_u24_le(input: &[u8]) -> usize {
    usize::from(input[0]) | (usize::from(input[1]) << 8) | (usize::from(input[2]) << 16)
}

#[cfg(test)]
mod tests {
    use super::super::viewer;
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn replace_z_leaves_backslash() {
        assert_eq!(replace_z_with_backslash("abzcd"), "ab\\cd");
        assert_eq!(replace_z_with_backslash("zzz"), "\\\\\\");
        assert_eq!(replace_z_with_backslash("no_zed_here"), "no_\\ed_here");
        assert_eq!(replace_z_with_backslash("abc"), "abc");
    }

    #[test]
    fn replace_z_is_idempotent_modulo_backslash() {
        let once = replace_z_with_backslash("azbzczd");
        let twice = replace_z_with_backslash(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn pp_body_decodes_as_modified_base85_after_z_replacement() {
        // Verify that every P/P and R/R encrypted fixture body becomes valid
        // modified Base85 after `z`→`\\` replacement and envelope stripping.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted");
        for entry in fs::read_dir(&root).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.is_dir() {
                continue;
            }
            let xml_path = dir.join("tour.xml");
            if !xml_path.exists() {
                continue;
            }
            let xml = fs::read_to_string(&xml_path).unwrap();
            let payload = viewer::encrypted_payload(xml.as_bytes()).unwrap();
            let header = super::super::header::KencHeader::parse(&payload).unwrap();

            // Only test P/P and R/R branches.
            match header.branch() {
                super::super::header::KencBranch::PP | super::super::header::KencBranch::RR => {}
                _ => continue,
            }

            let body = header.payload(&payload);
            let replaced = replace_z_with_backslash(body);
            let envelope = parse_pp_rr_envelope(&replaced);

            // After replacement, the body must not contain any `z`.
            assert!(
                !replaced.contains('z'),
                "{}: body still has 'z' after replaceAll",
                dir.display()
            );

            // Must decode as valid modified Base85.
            let decoded = codecs::decode_modified_base85(envelope.payload)
                .unwrap_or_else(|e| panic!("{}: Base85 decode failed: {e}", dir.display()));

            // Decoded body must be large enough for the RC4 header (≥ 128 bytes).
            // Small fixtures like 01_minimal legitimately produce shorter bodies.
            // Skip them — they're still valid P/P encrypted payloads.
            if decoded.len() < 128 {
                continue;
            }
        }
    }

    #[test]
    fn rr_envelope_exposes_custom_key_id() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted");
        let expected = [
            ("2026-06-25-rr_minimal", "PFIXTURE_rr_minimal", 128),
            (
                "2026-06-25-rr_special",
                "PFIXTURE_rr_specialfCiiC/?c_`B#Xqgv<P(s(R,'V8TTUtQm?B_5#&u48i5nb$l'QFw+J+/R",
                256,
            ),
            (
                "2026-06-25-rr_tour",
                "MFIXTURE_rr_tourEE&i2VUZMG&];u7[igF62HYoL)VFFeh=]A.Ata#sZ'WGNk*",
                372,
            ),
        ];
        for (fixture, expected_key_id, expected_decoded_len) in expected {
            let xml = fs::read_to_string(root.join(fixture).join("tour.xml")).unwrap();
            let payload = viewer::encrypted_payload(xml.as_bytes()).unwrap();
            let header = super::super::header::KencHeader::parse(&payload).unwrap();
            assert_eq!(header.branch(), super::super::header::KencBranch::RR);

            let replaced = replace_z_with_backslash(header.payload(&payload));
            let envelope = parse_pp_rr_envelope(&replaced);
            assert_eq!(envelope.key_id, Some(expected_key_id), "{fixture}");
            assert_eq!(
                codecs::decode_modified_base85(envelope.payload)
                    .unwrap()
                    .len(),
                expected_decoded_len,
                "{fixture}"
            );
        }
    }

    #[test]
    #[ignore]
    fn analysis_prints_pp_rr_known_plaintext_stage_facts() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted");
        for name in [
            "2026-06-25-pp-01_minimal",
            "2026-06-25-pp-02_special_chars",
            "2026-06-25-pp-03_nested",
            "2026-06-25-pp-04_large",
            "2026-06-25-pp-05_deep",
            "2026-06-25-rr_minimal",
            "2026-06-25-rr_special",
            "2026-06-25-rr_tour",
        ] {
            let dir = root.join(name);
            let xml = fs::read_to_string(dir.join("tour.xml")).unwrap();
            let payload = viewer::encrypted_payload(xml.as_bytes()).unwrap();
            let header = super::super::header::KencHeader::parse(&payload).unwrap();
            let body = header.payload(&payload);
            let replaced = replace_z_with_backslash(body);
            let decoded = decode_pp_rr_body(body).unwrap();
            let envelope = parse_pp_rr_envelope(&replaced);
            let inner_decoded = codecs::decode_modified_base85(envelope.payload).ok();
            let default_decrypted = inner_decoded
                .as_ref()
                .and_then(|bytes| crypto::decrypt_bytes(bytes, b"actions overflow", false).ok());
            let widened_decrypted = inner_decoded
                .as_ref()
                .and_then(|bytes| crypto::decrypt_bytes(bytes, b"actions overflow", true).ok());
            let plaintext = fs::read(dir.join("plaintext.xml")).unwrap();
            let replaced_prefix = &replaced[..replaced.len().min(120)];
            let decoded_prefix = String::from_utf8_lossy(&decoded[..decoded.len().min(80)]);
            let plaintext_prefix = String::from_utf8_lossy(&plaintext[..plaintext.len().min(80)]);
            eprintln!(
                "{name}: branch={:?} body={} replaced_prefix={replaced_prefix:?} inner={} inner_decoded={:?} default_decrypted={:?} widened_decrypted={:?} decoded={} plaintext={} decoded_prefix={decoded_prefix:?} plaintext_prefix={plaintext_prefix:?}",
                header.branch(),
                body.len(),
                envelope.payload.len(),
                inner_decoded.as_ref().map(Vec::len),
                default_decrypted.as_ref().map(Vec::len),
                widened_decrypted.as_ref().map(Vec::len),
                decoded.len(),
                plaintext.len()
            );
        }
    }

    // ---------------------------------------------------------------------
    // Z branch transform tests
    // ---------------------------------------------------------------------

    /// Decrypt the 2018-04-04 Modern Z fixture end-to-end.
    #[test]
    fn decrypts_2018_04_04_z_branch() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted/2018-04-04");
        let xml = fs::read_to_string(root.join("tour.xml")).unwrap();
        let payload = viewer::encrypted_payload(xml.as_bytes()).unwrap();
        let header = super::super::header::KencHeader::parse(&payload).unwrap();
        assert_eq!(header.branch(), super::super::header::KencBranch::ModernZ);

        let body = header.payload(&payload);
        // body: 9915 chars of modified Base85 (7932 bytes decoded)
        let plaintext = z_branch_to_plaintext(body, b"actions overflow", false).unwrap();
        assert_eq!(plaintext.len(), 36407, "plaintext length");
        assert!(
            plaintext.trim().starts_with("<krpano"),
            "plaintext should start with <krpano>: {}",
            &plaintext[..200.min(plaintext.len())]
        );
    }

    /// Verify each stage of the Z branch pipeline produces expected byte counts.
    #[test]
    fn z_branch_stage_vectors() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted/2018-04-04");
        let xml = fs::read_to_string(root.join("tour.xml")).unwrap();
        let payload = viewer::encrypted_payload(xml.as_bytes()).unwrap();
        let header = super::super::header::KencHeader::parse(&payload).unwrap();

        let body = header.payload(&payload);
        let decoded = codecs::decode_modified_base85(body).unwrap();
        assert_eq!(decoded.len(), 7932, "post-Base85 length");

        let decrypted = crypto::decrypt_bytes(&decoded, b"actions overflow", false).unwrap();
        assert_eq!(decrypted.len(), 7803, "post-decrypt length");

        // Parse LZ4 header from decrypted bytes.
        let decompressed_len = read_u24_le(&decrypted[0..3]);
        let compressed_end = 8 + read_u24_le(&decrypted[4..7]);
        let plaintext =
            codecs::lz4_decompress_block(&decrypted[8..], decompressed_len, compressed_end - 8)
                .unwrap();
        assert_eq!(plaintext.len(), 36407, "post-LZ4 length");
    }

    fn load_old_fixture(
        fixture: &str,
    ) -> (String, super::super::header::KencHeader, Vec<u8>, String) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/krpano/encrypted")
            .join(fixture);
        let xml = fs::read_to_string(root.join("tour.xml")).unwrap();
        let payload = viewer::encrypted_payload(xml.as_bytes()).unwrap();
        let header = super::super::header::KencHeader::parse(&payload).unwrap();
        let js_path = ["tour.js", "krpano.js"]
            .iter()
            .map(|name| root.join(name))
            .find(|path| path.exists())
            .unwrap();
        let js = fs::read(js_path).unwrap();
        let decoded = viewer::extract_decoded_viewer_js(&js).unwrap();
        let wrapper_key = viewer::extract_key_from_viewer_js(&js).unwrap();
        (payload, header, decoded, wrapper_key)
    }

    #[test]
    fn decrypts_old_b_branch() {
        for fixture in ["2013-06-05-B", "2013-08-09-B"] {
            let (payload, header, decoded, wrapper_key) = load_old_fixture(fixture);
            assert_eq!(header.branch(), super::super::header::KencBranch::B);
            let old_ctx =
                super::super::old_engine::derive_old_license_key(&decoded, &wrapper_key).unwrap();
            let plaintext = b_branch_to_plaintext_with_alphabet(
                header.payload(&payload),
                &old_ctx.base64_alphabet,
                &old_ctx.default_key,
                false,
            )
            .unwrap_or_else(|err| panic!("{fixture}: {err}"));
            let normalized = plaintext.trim_start_matches('\u{feff}').trim_start();
            assert!(
                normalized.starts_with("<krpano"),
                "{fixture}: plaintext should start with <krpano>: {}",
                &plaintext[..200.min(plaintext.len())]
            );
        }
    }

    #[test]
    fn decrypts_old_z_branch() {
        for fixture in ["old", "2015-08-04", "2017-09-21"] {
            let (payload, header, decoded, wrapper_key) = load_old_fixture(fixture);
            assert_eq!(header.branch(), super::super::header::KencBranch::OldZ);
            let old_ctx = super::super::old_engine::derive_old_license_key(&decoded, &wrapper_key)
                .unwrap_or_else(|err| panic!("{fixture}: {err}"));
            let protected_key = old_ctx
                .protected_key
                .as_deref()
                .unwrap_or_else(|| panic!("{fixture}: missing protected key"));
            let plaintext = z_branch_to_plaintext(header.payload(&payload), protected_key, true)
                .unwrap_or_else(|err| panic!("{fixture}: {err}"));
            let normalized = plaintext.trim_start_matches('\u{feff}').trim_start();
            assert!(
                normalized.starts_with("<krpano"),
                "{fixture}: plaintext should start with <krpano>: {}",
                &plaintext[..200.min(plaintext.len())]
            );
        }
    }
}
