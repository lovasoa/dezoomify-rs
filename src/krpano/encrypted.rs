use custom_error::custom_error;
use lazy_static::lazy_static;
use regex::Regex;

custom_error! {pub EncryptedKrpanoError
    MissingEncryptedPayload = "encrypted krpano XML did not contain an <encrypted> payload",
    MissingKey = "encrypted krpano XML needs the krpano viewer JavaScript decryption key",
    Unsupported = "encrypted krpano XML decryption is not implemented for this payload variant yet",
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

pub fn decrypt_xml(contents: &[u8], key: Option<&str>) -> Result<Vec<u8>, EncryptedKrpanoError> {
    let _payload = encrypted_payload(contents)?;
    let _key = key.ok_or(EncryptedKrpanoError::MissingKey)?;
    Err(EncryptedKrpanoError::Unsupported)
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
    }
}
