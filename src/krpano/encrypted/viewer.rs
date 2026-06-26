use lazy_static::lazy_static;
use regex::Regex;

use super::codecs;
use super::EncryptedKrpanoError;

lazy_static! {
    static ref ENCRYPTED_RE: Regex =
        Regex::new(r#"(?is)<encrypted>(?P<body>.*?)</encrypted>"#).unwrap();
    static ref CDATA_RE: Regex = Regex::new(r#"(?is)<!\[CDATA\[(?P<cdata>.*?)\]\]>"#).unwrap();
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

pub fn extract_key_from_viewer_js(contents: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(contents);
    log::debug!(
        "extract_key_from_viewer_js: scanning {} bytes for krp: wrapper key",
        contents.len()
    );
    let mut idx = 0;
    while let Some((literal, next_idx)) = next_js_string_literal(&text, idx) {
        idx = next_idx;
        if literal.starts_with("krp:") {
            log::debug!(
                "extract_key_from_viewer_js: found krp: key at offset {}, length {}",
                next_idx - literal.len() - 2,
                literal.len()
            );
            return Some(literal);
        }
    }
    log::debug!("extract_key_from_viewer_js: no krp: key found");
    None
}

pub fn extract_decoded_viewer_js(contents: &[u8]) -> Result<Vec<u8>, EncryptedKrpanoError> {
    log::debug!(
        "extract_decoded_viewer_js: scanning {} bytes for packed viewer",
        contents.len()
    );
    let text = String::from_utf8_lossy(contents);
    let mut idx = 0;
    let mut candidates = 0u32;
    while let Some((literal, next_idx)) = next_js_string_literal(&text, idx) {
        idx = next_idx;
        if !looks_like_modified_base85(&literal) {
            continue;
        }
        candidates += 1;
        match codecs::decode_packed_viewer_js_payload(&literal) {
            Ok(decoded) if looks_like_decoded_viewer_js(&decoded) => {
                log::debug!(
                    "extract_decoded_viewer_js: decoded packed viewer (candidate #{candidates}, raw={} chars, decoded={} bytes)",
                    literal.len(),
                    decoded.len()
                );
                return Ok(decoded);
            }
            Ok(decoded) => {
                log::debug!(
                    "extract_decoded_viewer_js: candidate #{candidates} decoded but doesn't look like viewer JS ({} bytes, prefix: {})",
                    decoded.len(),
                    String::from_utf8_lossy(&decoded[..200.min(decoded.len())])
                );
            }
            Err(e) => {
                log::debug!(
                    "extract_decoded_viewer_js: candidate #{candidates} ({len} chars) decode failed: {e}",
                    len = literal.len()
                );
            }
        }
    }
    log::debug!(
        "extract_decoded_viewer_js: no valid packed viewer found ({candidates} modified-Base85 candidates scanned)"
    );
    Err(EncryptedKrpanoError::MissingViewerJsPayload)
}

pub fn next_js_string_literal(text: &str, start: usize) -> Option<(String, usize)> {
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
                        let value =
                            (hex_digit(bytes[idx + 1])? << 4) | hex_digit(bytes[idx + 2])?;
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

pub fn looks_like_modified_base85(input: &str) -> bool {
    input.len() >= codecs::MIN_PACKED_VIEWER_PAYLOAD_LEN
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

pub fn looks_like_decoded_viewer_js(input: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(input) else {
        return false;
    };
    text.starts_with("function ")
        && (text.contains("loadpano") || text.contains("embedhtml5") || text.contains("krpano"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_and_concatenates_encrypted_cdata() {
        let xml = br#"<encrypted><![CDATA[KENCR]]><![CDATA[URRpayload]]></encrypted>"#;
        assert!(is_encrypted_xml(xml));
        assert_eq!(encrypted_payload(xml).unwrap(), "KENCRURRpayload");
    }

    #[test]
    fn extracts_krpano_decryption_key_from_viewer_js() {
        let js = br#"return function(t){r&&(h=r(),r=null);h(t,"krp:abc def")}"#;
        assert_eq!(
            extract_key_from_viewer_js(js),
            Some("krp:abc def".to_string())
        );
        let js2 = br"embedhtml5(e.params,'krp:xyz123')";
        assert_eq!(
            extract_key_from_viewer_js(js2),
            Some("krp:xyz123".to_string())
        );
    }
}
