use custom_error::custom_error;
use lazy_static::lazy_static;
use regex::Regex;

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

const PACKED_VIEWER_HEADER_LEN: usize = 8;
const MIN_PACKED_VIEWER_PAYLOAD_LEN: usize = 100;
const MAX_DECODED_VIEWER_JS_LEN: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KencHeader {
    /// The full eight-byte `KENC....` marker at the start of the payload.
    pub raw: String,
    /// First mode byte after `KENC`.
    pub mode: char,
    /// Encoding/compression byte after the mode.
    pub encoding: char,
    /// Key/source byte after the encoding.
    pub key_source: char,
    /// Final flag byte. Observed samples end with `R`.
    pub flags: char,
}

impl KencHeader {
    const LEN: usize = 8;

    pub fn parse(payload: &str) -> Result<Self, EncryptedKrpanoError> {
        let header = payload
            .get(..Self::LEN)
            .ok_or(EncryptedKrpanoError::HeaderTooShort { len: payload.len() })?;
        if !header.starts_with("KENC") {
            return Err(EncryptedKrpanoError::InvalidHeader {
                header: header.to_string(),
            });
        }
        let raw = header.to_string();
        let fields: Vec<_> = raw.chars().skip(4).collect();
        Ok(Self {
            raw,
            mode: fields[0],
            encoding: fields[1],
            key_source: fields[2],
            flags: fields[3],
        })
    }

    pub fn payload<'a>(&self, payload: &'a str) -> &'a str {
        &payload[Self::LEN..]
    }
}

#[allow(dead_code)]
fn decrypt_bytes(
    input: &[u8],
    key: &[u8],
    widened_key_index: bool,
) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let mut key_mask = 15usize;
    let prefix_len = 1usize << (key_mask / 2);
    if input.len() < prefix_len || key.is_empty() {
        return Err(EncryptedKrpanoError::InvalidByteCipherInput);
    }
    let encrypted_start = prefix_len + (usize::from(input[usize::from(b'A')]) & (key_mask >> 1));
    if encrypted_start > input.len() {
        return Err(EncryptedKrpanoError::InvalidByteCipherInput);
    }
    if widened_key_index {
        key_mask |= key_mask << 3;
    }

    let mut mixed_key = vec![0u8; prefix_len * 2];
    let mut out = 0;
    for idx in 0..prefix_len {
        mixed_key[out] = input[idx];
        mixed_key[out + 1] = key[(idx & key_mask) % key.len()];
        out += 2;
    }

    let mut state = [0u8; 256];
    for (idx, value) in state.iter_mut().enumerate() {
        *value = idx as u8;
    }
    let mut j = 0usize;
    for idx in 0..256 {
        j = (j + usize::from(state[idx]) + usize::from(mixed_key[idx])) & 255;
        state.swap(idx, j);
    }

    // krpano discards the first 256 bytes of the stream.
    let mut i = 0usize;
    j = 0;
    for _ in 0..256 {
        i = (i + 1) & 255;
        j = (j + usize::from(state[i])) & 255;
        state.swap(i, j);
    }

    let mut decrypted = Vec::with_capacity(input.len() - encrypted_start);
    for &byte in &input[encrypted_start..] {
        i = (i + 1) & 255;
        j = (j + usize::from(state[i])) & 255;
        let key_byte = state[(usize::from(state[i]) + usize::from(state[j])) & 255];
        decrypted.push(byte ^ key_byte);
        state.swap(i, j);
    }
    Ok(decrypted)
}

#[allow(dead_code)]
fn decode_modified_base85(input: &str) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let complete_len = input.len() / 5 * 5;
    let mut decoded = Vec::with_capacity(complete_len / 5 * 4);
    for chunk in input.as_bytes()[..complete_len].chunks_exact(5) {
        let mut value = 0u64;
        for &byte in chunk {
            let mut digit = byte
                .checked_sub(35)
                .ok_or(EncryptedKrpanoError::InvalidBase85Byte { byte })?;
            if digit > 56 {
                digit -= 1;
            }
            if digit >= 85 {
                return Err(EncryptedKrpanoError::InvalidBase85Byte { byte });
            }
            value = value * 85 + u64::from(digit);
        }
        decoded.extend_from_slice(&(value as u32).to_be_bytes());
    }
    Ok(decoded)
}

#[allow(dead_code)]
fn lz4_decompress_block(
    input: &[u8],
    decompressed_len: usize,
    compressed_end: usize,
) -> Result<Vec<u8>, EncryptedKrpanoError> {
    if compressed_end > input.len() {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }
    let mut src = 0;
    let mut output = Vec::with_capacity(decompressed_len);
    while src < compressed_end {
        let token = input[src];
        src += 1;

        let literal_len = read_lz4_len(token >> 4, input, &mut src, compressed_end)?;
        if src + literal_len > compressed_end {
            return Err(EncryptedKrpanoError::InvalidLz4Block);
        }
        output.extend_from_slice(&input[src..src + literal_len]);
        src += literal_len;
        if src == compressed_end {
            break;
        }

        if src + 2 > compressed_end {
            return Err(EncryptedKrpanoError::InvalidLz4Block);
        }
        let offset = usize::from(input[src]) | (usize::from(input[src + 1]) << 8);
        src += 2;
        if offset == 0 || offset > output.len() {
            return Err(EncryptedKrpanoError::InvalidLz4Block);
        }
        let match_len = read_lz4_len(token & 0x0f, input, &mut src, compressed_end)? + 4;
        for _ in 0..match_len {
            let byte = output[output.len() - offset];
            output.push(byte);
        }
    }
    if output.len() != decompressed_len {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }
    Ok(output)
}

#[allow(dead_code)]
fn read_lz4_len(
    nibble: u8,
    input: &[u8],
    src: &mut usize,
    compressed_end: usize,
) -> Result<usize, EncryptedKrpanoError> {
    let mut len = usize::from(nibble);
    if nibble == 15 {
        loop {
            if *src >= compressed_end {
                return Err(EncryptedKrpanoError::InvalidLz4Block);
            }
            let next = input[*src];
            *src += 1;
            len += usize::from(next);
            if next != 255 {
                break;
            }
        }
    }
    Ok(len)
}

#[allow(dead_code)]
fn decode_packed_viewer_js_payload(input: &str) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let packed = decode_modified_base85(input)?;
    if packed.len() < PACKED_VIEWER_HEADER_LEN {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }

    let decompressed_len = read_u24_le(&packed[0..3]);
    if decompressed_len > MAX_DECODED_VIEWER_JS_LEN {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }
    let compressed_end = PACKED_VIEWER_HEADER_LEN + read_u24_le(&packed[4..7]);
    if compressed_end > packed.len() {
        return Err(EncryptedKrpanoError::InvalidLz4Block);
    }

    lz4_decompress_block(
        &packed[PACKED_VIEWER_HEADER_LEN..],
        decompressed_len,
        compressed_end - PACKED_VIEWER_HEADER_LEN,
    )
}

fn read_u24_le(input: &[u8]) -> usize {
    usize::from(input[0]) | (usize::from(input[1]) << 8) | (usize::from(input[2]) << 16)
}

fn looks_like_modified_base85(input: &str) -> bool {
    input.len() >= MIN_PACKED_VIEWER_PAYLOAD_LEN
        && input.len() % 5 == 0
        && input.bytes().all(|byte| {
            byte.checked_sub(35)
                .map(|mut digit| {
                    if digit > 56 {
                        digit -= 1;
                    }
                    digit < 85
                })
                .unwrap_or(false)
        })
}

fn looks_like_decoded_viewer_js(input: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(input) else {
        return false;
    };
    text.starts_with("function ")
        && (text.contains("loadpano") || text.contains("embedhtml5") || text.contains("krpano"))
}

fn next_js_string_literal(text: &str, start: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut idx = start;
    while idx < bytes.len() {
        let quote = bytes[idx];
        if quote != b'"' && quote != b'\'' {
            idx += 1;
            continue;
        }

        idx += 1;
        let mut literal = String::new();
        while idx < bytes.len() {
            let byte = bytes[idx];
            if byte == quote {
                return Some((literal, idx + 1));
            }
            if byte == b'\\' {
                idx += 1;
                if idx >= bytes.len() {
                    return None;
                }
                match bytes[idx] {
                    b'x' => {
                        if idx + 2 >= bytes.len() {
                            return None;
                        }
                        let value = (hex_digit(bytes[idx + 1])? << 4) | hex_digit(bytes[idx + 2])?;
                        literal.push(char::from(value));
                        idx += 3;
                    }
                    b'u' => {
                        if idx + 4 >= bytes.len() {
                            return None;
                        }
                        let value = (u32::from(hex_digit(bytes[idx + 1])?) << 12)
                            | (u32::from(hex_digit(bytes[idx + 2])?) << 8)
                            | (u32::from(hex_digit(bytes[idx + 3])?) << 4)
                            | u32::from(hex_digit(bytes[idx + 4])?);
                        literal.push(char::from_u32(value)?);
                        idx += 5;
                    }
                    b'\r' => {
                        idx += 1;
                        if idx < bytes.len() && bytes[idx] == b'\n' {
                            idx += 1;
                        }
                    }
                    b'\n' => {
                        idx += 1;
                    }
                    b'b' => {
                        literal.push('\u{0008}');
                        idx += 1;
                    }
                    b'f' => {
                        literal.push('\u{000c}');
                        idx += 1;
                    }
                    b'n' => {
                        literal.push('\n');
                        idx += 1;
                    }
                    b'r' => {
                        literal.push('\r');
                        idx += 1;
                    }
                    b't' => {
                        literal.push('\t');
                        idx += 1;
                    }
                    b'v' => {
                        literal.push('\u{000b}');
                        idx += 1;
                    }
                    escaped => {
                        literal.push(char::from(escaped));
                        idx += 1;
                    }
                }
                continue;
            }
            literal.push(char::from(byte));
            idx += 1;
        }
        return None;
    }
    None
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

lazy_static! {
    static ref ENCRYPTED_RE: Regex =
        Regex::new(r#"(?is)<encrypted>(?P<body>.*?)</encrypted>"#).unwrap();
    static ref CDATA_RE: Regex = Regex::new(r#"(?is)<!\[CDATA\[(?P<cdata>.*?)\]\]>"#).unwrap();
    static ref KRPANO_KEY_RE: Regex =
        Regex::new(r#"(?s)\bh\s*\(\s*t\s*,\s*["'](?P<key>krp:[^"']+)["']\s*\)"#).unwrap();
}

pub fn is_encrypted_xml(contents: &[u8]) -> bool {
    let text = String::from_utf8_lossy(contents);
    ENCRYPTED_RE.is_match(&text)
}

pub fn encrypted_payload(contents: &[u8]) -> Result<String, EncryptedKrpanoError> {
    let text = String::from_utf8_lossy(contents);
    if !ENCRYPTED_RE.is_match(&text) {
        return Err(EncryptedKrpanoError::MissingEncryptedPayload);
    }
    let body = ENCRYPTED_RE
        .captures(&text)
        .and_then(|caps| caps.name("body"))
        .ok_or(EncryptedKrpanoError::MissingEncryptedPayload)?
        .as_str();
    let mut payload = String::new();
    for caps in CDATA_RE.captures_iter(body) {
        if let Some(cdata) = caps.name("cdata") {
            payload.push_str(cdata.as_str());
        }
    }
    if payload.is_empty() {
        payload.push_str(body.trim());
    }
    Ok(payload)
}

#[allow(dead_code)]
pub fn extract_key_from_viewer_js(contents: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(contents);
    KRPANO_KEY_RE
        .captures(&text)
        .and_then(|caps| caps.name("key"))
        .map(|m| m.as_str().to_string())
}

#[allow(dead_code)]
pub fn extract_decoded_viewer_js(contents: &[u8]) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let text = String::from_utf8_lossy(contents);
    let mut idx = 0;
    while let Some((literal, next_idx)) = next_js_string_literal(&text, idx) {
        idx = next_idx;
        if !looks_like_modified_base85(&literal) {
            continue;
        }
        if let Ok(decoded) = decode_packed_viewer_js_payload(&literal) {
            if looks_like_decoded_viewer_js(&decoded) {
                return Ok(decoded);
            }
        }
    }
    Err(EncryptedKrpanoError::MissingViewerJsPayload)
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

    #[test]
    fn detects_and_concatenates_encrypted_cdata() {
        let xml = br#"<encrypted><![CDATA[KENCR]]><![CDATA[URRpayload]]></encrypted>"#;
        assert!(is_encrypted_xml(xml));
        assert_eq!(encrypted_payload(xml).unwrap(), "KENCRURRpayload");
    }

    #[test]
    fn parses_known_kenc_headers() {
        let public = KencHeader::parse("KENCPUPRpayload").unwrap();
        assert_eq!(public.payload("KENCPUPRpayload"), "payload");
        assert_eq!(
            public,
            KencHeader {
                raw: "KENCPUPR".to_string(),
                mode: 'P',
                encoding: 'U',
                key_source: 'P',
                flags: 'R',
            }
        );
        assert_eq!(
            KencHeader::parse("KENCRURRpayload").unwrap(),
            KencHeader {
                raw: "KENCRURR".to_string(),
                mode: 'R',
                encoding: 'U',
                key_source: 'R',
                flags: 'R',
            }
        );
    }

    #[test]
    fn rejects_invalid_kenc_headers() {
        assert!(matches!(
            KencHeader::parse("short"),
            Err(EncryptedKrpanoError::HeaderTooShort { len: 5 })
        ));
        assert!(matches!(
            KencHeader::parse("NOTKENC!payload"),
            Err(EncryptedKrpanoError::InvalidHeader { .. })
        ));
    }

    #[test]
    fn decrypts_byte_cipher_payload() {
        let key = b"test-key";
        let plaintext = b"plain krpano bytes";
        let prefix_len = 128;
        let mut encrypted = vec![0u8; prefix_len];
        encrypted[usize::from(b'A')] = 0;
        let stream = decrypt_bytes(&encrypted, key, true).unwrap();
        assert!(stream.is_empty());

        let mut ciphertext_source = encrypted.clone();
        ciphertext_source.extend(std::iter::repeat_n(0, plaintext.len()));
        let keystream = decrypt_bytes(&ciphertext_source, key, true).unwrap();
        let ciphertext: Vec<_> = plaintext
            .iter()
            .zip(keystream.iter())
            .map(|(&plain, &stream)| plain ^ stream)
            .collect();

        let mut encrypted = encrypted;
        encrypted.extend_from_slice(&ciphertext);
        assert_eq!(decrypt_bytes(&encrypted, key, true).unwrap(), plaintext);
    }

    #[test]
    fn decodes_modified_base85_chunks() {
        assert_eq!(decode_modified_base85("7vgt.").unwrap(), b"ABCD");
    }

    #[test]
    fn decodes_lz4_literal_only_block() {
        assert_eq!(
            lz4_decompress_block(&[0x30, b'a', b'b', b'c'], 3, 4).unwrap(),
            b"abc"
        );
    }

    #[test]
    fn decodes_lz4_back_reference_block() {
        assert_eq!(
            lz4_decompress_block(&[0x32, b'a', b'b', b'c', 3, 0], 9, 6).unwrap(),
            b"abcabcabc"
        );
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

    fn viewer_js_path(dir: &Path) -> Option<PathBuf> {
        ["tour.js", "krpano.js"]
            .into_iter()
            .map(|name| dir.join(name))
            .find(|path| path.exists())
    }

    #[test]
    fn extracts_krpano_decryption_key_from_viewer_js() {
        let js = br#"return function(t){r&&(h=r(),r=null);h(t,"krp:abc def")}"#;
        assert_eq!(
            extract_key_from_viewer_js(js),
            Some("krp:abc def".to_string())
        );
    }
}
