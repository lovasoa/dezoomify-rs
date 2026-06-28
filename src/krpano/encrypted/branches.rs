use super::EncryptedKrpanoError;
use super::codecs;
use super::crypto;

// ---------------------------------------------------------------------------
// Subdiv body prefix — the `%*` / `$*<key-id>@` envelope in P/P and R/R bodies
// ---------------------------------------------------------------------------

/// The 1.24 subdiv body starts with a prefix that is stripped before the
/// main pipeline.  This is distinct from the branch-5 2023/2024 path which
/// reads the body directly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub struct SubdivBodyPrefix<'a> {
    pub key_id: Option<&'a str>,
    pub payload: &'a str,
}

#[allow(dead_code)]
pub fn parse_subdiv_body_prefix(replaced_body: &str) -> SubdivBodyPrefix<'_> {
    SubdivBodyPrefix::parse(replaced_body)
}

impl<'a> SubdivBodyPrefix<'a> {
    pub fn parse(replaced_body: &'a str) -> Self {
        if let Some(payload) = replaced_body.strip_prefix("%*") {
            return Self {
                key_id: None,
                payload,
            };
        }
        if let Some(rest) = replaced_body.strip_prefix("$*")
            && let Some((key_id, payload)) = rest.split_once('@') {
                return Self {
                    key_id: Some(key_id),
                    payload,
                };
            }
        Self {
            key_id: None,
            payload: replaced_body,
        }
    }
}

// ---------------------------------------------------------------------------
// Z branch (ClassicZ cipher) — Modified Base85 → RC4 → LZ4 → UTF-8
// ---------------------------------------------------------------------------

/// Decrypt a ClassicZ-cipher encrypted body into raw bytes.
///
/// Pipeline: modified Base85 decode → byte decrypt (RC4-like) →
/// parse LZ4 block header → LZ4 decompress.
///
/// The first 128 bytes of the Base85-decoded data serve as the RC4
/// key-mixing prefix; the remainder is LZ4-compressed plaintext.
/// The LZ4 block carries an 8-byte header: 3-byte LE decompressed length,
/// then at offset 4 a 3-byte LE compressed length.
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
    if decompressed_len == 0 || decompressed_len > codecs::MAX_DECODED_VIEWER_JS_LEN {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }
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

/// Decrypt a ClassicZ-cipher body into a UTF-8 plaintext string.
pub fn z_branch_to_plaintext(
    body: &str,
    key: &[u8],
    widened: bool,
) -> Result<String, EncryptedKrpanoError> {
    let decrypted = decrypt_z_branch(body, key, widened)?;
    String::from_utf8(decrypted).map_err(|_| EncryptedKrpanoError::InvalidUtf8)
}

// ---------------------------------------------------------------------------
// B branch (ClassicB cipher) — Base64 → RC4 → UTF-8
// ---------------------------------------------------------------------------

pub fn b_branch_to_plaintext_with_alphabet(
    body: &str,
    alphabet: &str,
    key: &[u8],
    widened: bool,
) -> Result<String, EncryptedKrpanoError> {
    let decoded = decode_custom_base64(body, alphabet)?;
    let decrypted = crypto::decrypt_bytes(&decoded, key, widened)?;
    String::from_utf8(decrypted).map_err(|_| EncryptedKrpanoError::InvalidUtf8)
}

// ---------------------------------------------------------------------------
// Legacy R/R analysis branch (test-only, superseded by we.subdiv branch 5)
// ---------------------------------------------------------------------------

/// Attempt the Z-like pipeline on subdiv bodies.
/// Only used as regression coverage; the real subdiv path is branch 5.
#[allow(dead_code)]
pub(crate) fn decrypt_subdiv_via_classic_pipeline(
    body: &str,
    key: &[u8],
) -> Result<String, EncryptedKrpanoError> {
    let replaced = body.replace('z', "\\");
    let prefix = parse_subdiv_body_prefix(&replaced);
    let decoded = codecs::decode_modified_base85(prefix.payload)?;
    let decrypted = crypto::decrypt_bytes(&decoded, key, true)?;

    if decrypted.len() < codecs::PACKED_VIEWER_HEADER_LEN {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }

    let decompressed_len = read_u24_le(&decrypted[0..3]);
    if decompressed_len == 0 || decompressed_len > codecs::MAX_DECODED_VIEWER_JS_LEN {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }
    let compressed_end = codecs::PACKED_VIEWER_HEADER_LEN + read_u24_le(&decrypted[4..7]);
    if compressed_end > decrypted.len() {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }

    let decompressed = codecs::lz4_decompress_block(
        &decrypted[codecs::PACKED_VIEWER_HEADER_LEN..],
        decompressed_len,
        compressed_end - codecs::PACKED_VIEWER_HEADER_LEN,
    )?;
    String::from_utf8(decompressed).map_err(|_| EncryptedKrpanoError::InvalidUtf8)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn decode_custom_base64(input: &str, alphabet: &str) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let alphabet: Vec<char> = alphabet.chars().collect();
    if alphabet.len() < 65 {
        return Err(EncryptedKrpanoError::ClassicBAlphabetTooShort {
            len: alphabet.len(),
        });
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut chars = input.chars();
    while let (Some(a), Some(b), Some(c), Some(d)) =
        (chars.next(), chars.next(), chars.next(), chars.next())
    {
        let a = alphabet
            .iter()
            .position(|&ch| ch == a)
            .ok_or(EncryptedKrpanoError::ClassicBCharNotFound {
                ch: a,
                alphabet_len: alphabet.len(),
            })?;
        let b = alphabet
            .iter()
            .position(|&ch| ch == b)
            .ok_or(EncryptedKrpanoError::ClassicBCharNotFound {
                ch: b,
                alphabet_len: alphabet.len(),
            })?;
        let c = alphabet
            .iter()
            .position(|&ch| ch == c)
            .ok_or(EncryptedKrpanoError::ClassicBCharNotFound {
                ch: c,
                alphabet_len: alphabet.len(),
            })?;
        let d = alphabet
            .iter()
            .position(|&ch| ch == d)
            .ok_or(EncryptedKrpanoError::ClassicBCharNotFound {
                ch: d,
                alphabet_len: alphabet.len(),
            })?;
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

// ---------------------------------------------------------------------------
// 1.24 Subdiv diagnostic (Base85 → RC4 → LZ4 → UTF-8) — for 2026 fixtures
// ---------------------------------------------------------------------------

/// Diagnostic: try the Base85→RC4→LZ4 pipeline on a 2026 1.24 subdiv body.
#[allow(dead_code)]
pub fn diagnose_subdiv_1_24_body(body: &str, key: &[u8]) -> Result<String, EncryptedKrpanoError> {
    let replaced = body.replace('z', "\\");
    eprintln!("  replaced body len={}", replaced.len());

    let prefix = parse_subdiv_body_prefix(&replaced);
    eprintln!(
        "  prefix key_id={:?}, payload len={}",
        prefix.key_id,
        prefix.payload.len()
    );

    // Try BE first (matches JS inline Base85: >>> 24)
    match codecs::decode_modified_base85(prefix.payload) {
        Ok(decoded) => {
            eprintln!("  BE Base85 decoded len={}", decoded.len());
            // Try widened RC4 first (the JS engine uses widened for Subdiv)
            match crypto::decrypt_bytes(&decoded, key, true) {
                Ok(decrypted) => {
                    eprintln!("  BE RC4(wide) decrypted len={}", decrypted.len());
                    if decrypted.len() >= 8 {
                        let decomp_len = read_u24_le(&decrypted[0..3]);
                        let comp_end = 8 + read_u24_le(&decrypted[4..7]);
                        eprintln!(
                            "  LZ4 header: decomp_len={decomp_len}, comp_end={comp_end}, data_len={}",
                            decrypted.len()
                        );
                        if decomp_len > 0
                            && decomp_len < 10_000_000
                            && comp_end > 8
                            && comp_end <= decrypted.len() + 128
                        {
                            return codecs::lz4_decompress_block(
                                &decrypted,
                                decomp_len,
                                comp_end.min(decrypted.len()),
                            )
                            .and_then(|d| {
                                String::from_utf8(d).map_err(|_| EncryptedKrpanoError::InvalidUtf8)
                            });
                        }
                    }
                }
                Err(e) => eprintln!("  BE RC4(wide) failed: {e:?}"),
            }
            // Try narrow RC4
            match crypto::decrypt_bytes(&decoded, key, false) {
                Ok(decrypted) => {
                    eprintln!("  BE RC4(narrow) decrypted len={}", decrypted.len());
                    if decrypted.len() >= 8 {
                        let decomp_len = read_u24_le(&decrypted[0..3]);
                        let comp_end = 8 + read_u24_le(&decrypted[4..7]);
                        eprintln!("  LZ4 header: decomp_len={decomp_len}, comp_end={comp_end}");
                        if decomp_len > 0
                            && decomp_len < 10_000_000
                            && comp_end > 8
                            && comp_end <= decrypted.len() + 128
                        {
                            return codecs::lz4_decompress_block(
                                &decrypted,
                                decomp_len,
                                comp_end.min(decrypted.len()),
                            )
                            .and_then(|d| {
                                String::from_utf8(d).map_err(|_| EncryptedKrpanoError::InvalidUtf8)
                            });
                        }
                    }
                }
                Err(e) => eprintln!("  BE RC4(narrow) failed: {e:?}"),
            }
        }
        Err(e) => eprintln!("  BE Base85 failed: {e:?}"),
    }

    // Try LE
    match codecs::decode_modified_base85_little_endian(prefix.payload) {
        Ok(decoded) => {
            eprintln!("  LE Base85 decoded len={}", decoded.len());
            match crypto::decrypt_bytes(&decoded, key, true) {
                Ok(decrypted) => {
                    eprintln!("  LE RC4(wide) decrypted len={}", decrypted.len());
                    if decrypted.len() >= 8 {
                        let decomp_len = read_u24_le(&decrypted[0..3]);
                        let comp_end = 8 + read_u24_le(&decrypted[4..7]);
                        eprintln!("  LZ4 header: decomp_len={decomp_len}, comp_end={comp_end}");
                        if decomp_len > 0
                            && decomp_len < 10_000_000
                            && comp_end > 8
                            && comp_end <= decrypted.len() + 128
                        {
                            return codecs::lz4_decompress_block(
                                &decrypted,
                                decomp_len,
                                comp_end.min(decrypted.len()),
                            )
                            .and_then(|d| {
                                String::from_utf8(d).map_err(|_| EncryptedKrpanoError::InvalidUtf8)
                            });
                        }
                    }
                }
                Err(e) => eprintln!("  LE RC4(wide) failed: {e:?}"),
            }
        }
        Err(e) => eprintln!("  LE Base85 failed: {e:?}"),
    }

    Err(EncryptedKrpanoError::InvalidUtf8)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::viewer;
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn subdiv_body_prefix_parses_pp_and_rr() {
        let pp = SubdivBodyPrefix::parse("%*base85payload");
        assert_eq!(pp.key_id, None);
        assert_eq!(pp.payload, "base85payload");

        let rr = SubdivBodyPrefix::parse("$*key123@base85payload");
        assert_eq!(rr.key_id, Some("key123"));
        assert_eq!(rr.payload, "base85payload");
    }

    #[test]
    fn decrypts_2018_04_04_z_branch() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/krpano/encrypted/2018-04-04");
        let xml = fs::read_to_string(root.join("tour.xml")).unwrap();
        let payload = viewer::encrypted_payload(xml.as_bytes()).unwrap();
        let header = super::super::header::KencHeader::parse(&payload).unwrap();
        let body = header.payload(&payload);

        let plaintext = z_branch_to_plaintext(body, b"actions overflow", false).unwrap();
        assert!(plaintext.trim_start().starts_with("<krpano"));
    }

    #[test]
    fn decrypts_old_z_branch() {
        for fixture in ["old", "2015-08-04", "2017-09-21"] {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("testdata/krpano/encrypted")
                .join(fixture);
            let xml = fs::read_to_string(root.join("tour.xml")).unwrap();
            let payload = viewer::encrypted_payload(xml.as_bytes()).unwrap();
            let header = super::super::header::KencHeader::parse(&payload).unwrap();
            let body = header.payload(&payload);

            // Derive the old engine key
            let js = fs::read(
                ["tour.js", "krpano.js"]
                    .iter()
                    .map(|name| root.join(name))
                    .find(|p| p.exists())
                    .unwrap(),
            )
            .unwrap();
            let decoded = viewer::extract_decoded_viewer_js(&js).unwrap();
            let wrapper_key = viewer::extract_key_from_viewer_js(&js).unwrap();
            let ctx =
                super::super::old_engine::derive_old_license_key(&decoded, &wrapper_key).unwrap();
            let key = ctx.protected_key.as_ref().unwrap();

            let plaintext = z_branch_to_plaintext(body, key, true).unwrap();
            let normalized = plaintext.trim_start_matches('\u{feff}').trim_start();
            assert!(
                normalized.starts_with("<krpano"),
                "{fixture}: bad plaintext prefix: {normalized:?}"
            );
        }
    }

    #[test]
    fn decrypts_old_b_branch() {
        for fixture in ["2013-06-05-B", "2013-08-09-B"] {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("testdata/krpano/encrypted")
                .join(fixture);
            let xml = fs::read_to_string(root.join("tour.xml")).unwrap();
            let payload = viewer::encrypted_payload(xml.as_bytes()).unwrap();
            let header = super::super::header::KencHeader::parse(&payload).unwrap();
            let body = header.payload(&payload);

            let js = fs::read(
                ["tour.js", "krpano.js"]
                    .iter()
                    .map(|name| root.join(name))
                    .find(|p| p.exists())
                    .unwrap(),
            )
            .unwrap();
            let decoded = viewer::extract_decoded_viewer_js(&js).unwrap();
            let wrapper_key = viewer::extract_key_from_viewer_js(&js).unwrap();
            let ctx =
                super::super::old_engine::derive_old_license_key(&decoded, &wrapper_key).unwrap();

            let key = &ctx.default_key;
            let plaintext =
                b_branch_to_plaintext_with_alphabet(body, &ctx.base64_alphabet, key, false)
                    .unwrap();
            assert!(
                plaintext.trim_start().starts_with("<krpano"),
                "{fixture}: bad plaintext prefix"
            );
        }
    }

    /// Diagnostic: try branch 5 directly on 2026 PP/RR fixtures
    #[test]
    fn diagnose_2026_subdiv_fixtures() {
        use super::super::modern_engine;

        let fixtures = ["2026-06-25-pp-01_minimal", "2026-06-25-rr_minimal"];

        for name in fixtures {
            eprintln!("\n=== {name} ===");
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("testdata/krpano/encrypted")
                .join(name);
            let xml = fs::read_to_string(root.join("tour.xml")).unwrap();
            let payload = super::super::viewer::encrypted_payload(xml.as_bytes()).unwrap();
            let body = super::super::header::KencHeader::parse(&payload)
                .unwrap()
                .payload(&payload);
            let d = body.as_bytes();
            eprintln!(
                "  body len={} d[0]={} d[1]={} d[2]={}",
                body.len(),
                d[0],
                d[1],
                d.get(2).copied().unwrap_or(0)
            );

            let js = fs::read(root.join("tour.js")).unwrap();
            let decoded_engine = super::super::viewer::extract_decoded_viewer_js(&js).unwrap();
            let wrapper_key = super::super::viewer::extract_key_from_viewer_js(&js).unwrap();
            let ctx = modern_engine::extract_modern_context(&decoded_engine, &wrapper_key).unwrap();

            let row = ctx
                .rows
                .iter()
                .find(|r| r.iter().copied().eq("krpano".bytes().map(u16::from)))
                .unwrap();
            let g = i64::from(row[5]) / 3;
            eprintln!("  krpano row[5]={} g={g}", row[5]);

            let replaced = body.replace(&ctx.replacement_token, "\\");
            if name.contains("pp") {
                match modern_engine::subdiv_branch5_decode(&replaced, row, None, None) {
                    Ok(text) => eprintln!(
                        "  OK plaintext ({} bytes): {:?}",
                        text.len(),
                        &text[..(<str as AsRef<str>>::as_ref(&text)).len().min(200)]
                    ),
                    Err(e) => eprintln!("  FAIL: {e:?}"),
                }
            } else {
                let mf = modern_engine::build_mf_table(&ctx).unwrap_or_default();
                match modern_engine::subdiv_branch5_decode(&replaced, row, None, Some(&mf)) {
                    Ok(text) => eprintln!(
                        "  OK plaintext ({} bytes): {:?}",
                        text.len(),
                        &text[..(<str as AsRef<str>>::as_ref(&text)).len().min(200)]
                    ),
                    Err(e) => eprintln!("  FAIL: {e:?}"),
                }
            }
        }
    }
}
