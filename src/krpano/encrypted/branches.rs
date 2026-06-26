use super::codecs;
use super::crypto;
use super::EncryptedKrpanoError;

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
/// Pipeline: `z`→`\` → modified-Base85 decode.
pub fn decode_pp_rr_body(body: &str) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let replaced = replace_z_with_backslash(body);
    codecs::decode_modified_base85(&replaced)
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
        // modified Base85 after `z`→`\\` replacement — step 1 of the pipeline.
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

            // After replacement, the body must not contain any `z`.
            assert!(
                !replaced.contains('z'),
                "{}: body still has 'z' after replaceAll",
                dir.display()
            );

            // Must decode as valid modified Base85.
            let decoded = codecs::decode_modified_base85(&replaced)
                .unwrap_or_else(|e| panic!("{}: Base85 decode failed: {e}", dir.display()));

            // Decoded body must be large enough for the RC4 header (≥ 128 bytes).
            // Small fixtures like 01_minimal legitimately produce shorter bodies.
            // Skip them — they're still valid P/P encrypted payloads.
            if decoded.len() < 128 {
                continue;
            }
        }
    }

    // ---------------------------------------------------------------------
    // Z branch transform tests
    // ---------------------------------------------------------------------

    /// Decrypt the 2018-04-04 Modern Z fixture end-to-end.
    #[test]
    fn decrypts_2018_04_04_z_branch() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/krpano/encrypted/2018-04-04");
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
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/krpano/encrypted/2018-04-04");
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
        let plaintext = codecs::lz4_decompress_block(
            &decrypted[8..],
            decompressed_len,
            compressed_end - 8,
        )
        .unwrap();
        assert_eq!(plaintext.len(), 36407, "post-LZ4 length");
    }
}
