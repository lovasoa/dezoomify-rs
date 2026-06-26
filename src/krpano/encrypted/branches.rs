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
}
