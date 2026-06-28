//! Old krpano engine — key extraction from decoded engine JS and wrapper.
//!
//! Old engines (pre-2018) store constants in a literal `_[]` string table and a
//! hidden Base64 license blob, both unpacked from the `krp:` wrapper string.
//!
//! The wrapper string is decoded with a reverse-substitution cipher (salted,
//! rolling-checksummed) into two pieces:
//!
//! 1. **`_[]` rows** — a table of pipe-delimited strings.  Row 188 carries
//!    license field tags (e.g. `xx=lz=rg=ma=dm=ed=eu=ek=rd=pt=id=`).  Row
//!    references near the byte-helper function carry the default key and the
//!    Base64 alphabet used by the B cipher.
//!
//! 2. **License blob** — a hidden Base64-encoded string of semicolon-separated
//!    `key=value` records.  The record whose tag matches the field extracted
//!    from row 188 (e.g. `ek=`) is the **protected key**.  The engine's
//!    `pc.init` function processes this record in `case 7` of a switch
//!    statement: it Base64-decodes the value, computes a `ck=` checksum,
//!    looks up each character via `charCodeAt(i) & 255`, and pads the result
//!    to 128 characters.

use base64::Engine;

use super::EncryptedKrpanoError;

// ---------------------------------------------------------------------------
// Engine trait
// ---------------------------------------------------------------------------

/// Context produced by key derivation for a given engine family.
///
/// Both old and modern engines produce a context that the branch transform
/// reads to obtain the decryption key and any auxiliary data (Base64
/// alphabet for ClassicB, replacement token for Subdiv, etc.).
#[allow(dead_code)]
pub trait EngineContext: Clone + std::fmt::Debug {
    /// The default (non-license) key used when the header's cipher mode is
    /// `Public`.
    fn default_key(&self) -> &[u8];

    /// The license-derived key used when the cipher mode is `Protected`,
    /// or `None` if the engine does not carry a license.
    fn protected_key(&self) -> Option<&[u8]>;
}

/// Key derivation for an engine family.
#[allow(dead_code)]
pub trait KeyDerivation {
    type Ctx: EngineContext;

    /// Detect whether this engine family matches the decoded engine JS.
    fn matches(&self, decoded_engine: &str) -> bool;

    /// Derive the engine context from the decoded engine and wrapper key.
    fn derive(
        &self,
        decoded_engine: &[u8],
        wrapper_key: &str,
    ) -> Result<Self::Ctx, EncryptedKrpanoError>;
}

// ---------------------------------------------------------------------------
// Old engine context
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OldEngineContext {
    pub default_key: Vec<u8>,
    pub protected_key: Option<Vec<u8>>,
    pub base64_alphabet: String,
    pub key_variable: String,
}

impl EngineContext for OldEngineContext {
    fn default_key(&self) -> &[u8] {
        &self.default_key
    }

    fn protected_key(&self) -> Option<&[u8]> {
        self.protected_key.as_deref()
    }
}

// ---------------------------------------------------------------------------
// Old engine key derivation
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub struct OldEngine;

impl KeyDerivation for OldEngine {
    type Ctx = OldEngineContext;

    fn matches(&self, decoded_engine: &str) -> bool {
        decoded_engine.contains("KENC")
    }

    fn derive(
        &self,
        decoded_engine: &[u8],
        wrapper_key: &str,
    ) -> Result<Self::Ctx, EncryptedKrpanoError> {
        derive_old_license_key(decoded_engine, wrapper_key)
    }
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

struct OldWrapperPayload {
    rows: Vec<String>,
    license_blob: String,
}

/// Derive the old-engine keys from the decoded engine and wrapper string.
pub fn derive_old_license_key(
    decoded_engine: &[u8],
    wrapper_key: &str,
) -> Result<OldEngineContext, EncryptedKrpanoError> {
    let decoded_engine =
        std::str::from_utf8(decoded_engine).map_err(|_| EncryptedKrpanoError::InvalidUtf8)?;

    let unpacked = unpack_old_wrapper(wrapper_key)?;
    let key_tag = unpacked
        .rows
        .get(188)
        .and_then(|case_tags| case_tags.get(21..24))
        .filter(|tag| tag.ends_with('='))
        .unwrap_or("ek=");
    let default_key = find_old_default_key_row_index(decoded_engine)
        .and_then(|index| unpacked.rows.get(index))
        .map(|key| key.as_bytes().to_vec())
        .unwrap_or_default();
    let base64_alphabet = find_old_base64_alphabet_row_index(decoded_engine)
        .and_then(|index| unpacked.rows.get(index).cloned())
        .filter(|alpha| alpha.len() >= 65)
        .or_else(|| {
            // Fallback: scan all rows for a row that looks like a Base64 alphabet
            unpacked
                .rows
                .iter()
                .find(|row| row.len() >= 65 && row.starts_with("ABCDEFGHIJKLMNOPQRSTUVWXYZ"))
                .cloned()
        })
        .unwrap_or_default();
    let protected_key = extract_license_record(&unpacked.license_blob, key_tag)
        .ok()
        .map(String::into_bytes);

    Ok(OldEngineContext {
        default_key,
        protected_key,
        base64_alphabet,
        key_variable: find_old_key_variable(decoded_engine),
    })
}

fn find_old_default_key_row_index(decoded_engine: &str) -> Option<usize> {
    let marker_pos = decoded_engine
        .find("String(e).charCodeAt")
        .or_else(|| decoded_engine.find("String(h).charCodeAt"))?;
    let before_marker = &decoded_engine[..marker_pos];
    let row_ref_pos = before_marker.rfind("=_[")? + 3;
    let digits_end = before_marker[row_ref_pos..]
        .find(']')
        .map(|end| row_ref_pos + end)?;
    before_marker[row_ref_pos..digits_end].parse().ok()
}

fn find_old_base64_alphabet_row_index(decoded_engine: &str) -> Option<usize> {
    let marker_pos = decoded_engine.find("b64u8=function")?;
    let before_marker = &decoded_engine[..marker_pos];
    let row_ref_pos = before_marker.rfind("=_[")? + 3;
    let digits_end = before_marker[row_ref_pos..]
        .find(']')
        .map(|end| row_ref_pos + end)?;
    before_marker[row_ref_pos..digits_end].parse().ok()
}

fn find_old_key_variable(decoded_engine: &str) -> String {
    for variable in ["Pd", "od", "pe"] {
        if decoded_engine.contains(&format!("{variable}=null"))
            || decoded_engine.contains(&format!("var {variable}"))
        {
            return variable.to_string();
        }
    }
    "unknown".to_string()
}

/// Unpack the `krp:` wrapper string into the `_[]` row table and the hidden
/// license blob.  The cipher is a reverse-substitution with a per-fixture
/// salt (byte 4), a fixed shuffle array, and a rolling checksum.
fn unpack_old_wrapper(wrapper_key: &str) -> Result<OldWrapperPayload, EncryptedKrpanoError> {
    let bytes = wrapper_key.as_bytes();
    if bytes.len() < 8 || !wrapper_key.starts_with("krp:") {
        return Err(EncryptedKrpanoError::MissingKey);
    }

    let mut rows = Vec::new();
    let mut current = String::new();
    let mut license_blob = String::new();
    let mut row_run_len = 1usize;
    let mut hidden_toggle = 0u8;
    let salt = i32::from(bytes[4]);
    let mut rolling = salt;
    let shuffle = [1, 48, 55, 53, 38, 51, 52, 3];

    let payload_end = bytes.len() - 3;
    for (idx, &byte) in bytes.iter().enumerate().take(payload_end).skip(5) {
        let mut value = i32::from(byte);
        if value >= 92 {
            value -= 1;
        }
        if value >= 34 {
            value -= 1;
        }
        value -= 32;
        value = (value + 3 * idx as i32 + 59 + shuffle[idx & 7] + rolling).rem_euclid(93);
        rolling = (23 * rolling + value).rem_euclid(32749);
        value += 32;

        if value == i32::from(b'|') {
            if row_run_len == 0 {
                hidden_toggle ^= 1;
            } else if hidden_toggle == 1 {
                hidden_toggle = 0;
            } else {
                rows.push(std::mem::take(&mut current));
                row_run_len = 0;
            }
            continue;
        }

        let ch = char::from_u32(value as u32).ok_or(EncryptedKrpanoError::MissingKey)?;
        if hidden_toggle == 0 {
            current.push(ch);
        } else {
            license_blob.push(ch);
        }
        row_run_len += 1;
    }

    if row_run_len > 0 {
        rows.push(current);
    }

    let mut checksum = 0i32;
    for &byte in &bytes[payload_end..] {
        checksum = (checksum << 5) | (i32::from(byte) - 53);
    }
    if checksum != rolling {
        return Err(EncryptedKrpanoError::MissingKey);
    }

    Ok(OldWrapperPayload { rows, license_blob })
}

/// Extract the protected key from the license blob's `case 7` record.
///
/// The engine's `pc.init` function processes license records in a switch
/// statement.  Case 7 (the 8th branch) handles the XML encryption key:
/// it Base64-decodes the value, validates a `ck=` checksum, maps each
/// character through `charCodeAt(i) & 255`, and pads to 128 characters
/// by cycling through the key.
fn extract_license_record(
    license_blob: &str,
    key_tag: &str,
) -> Result<String, EncryptedKrpanoError> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(license_blob)
        .map_err(|_| EncryptedKrpanoError::MissingKey)?;
    let decoded = String::from_utf8(decoded).map_err(|_| EncryptedKrpanoError::MissingKey)?;
    let fields: Vec<&str> = decoded
        .split(';')
        .filter(|field| !field.is_empty())
        .collect();
    if fields.len() < 2 {
        return Err(EncryptedKrpanoError::MissingKey);
    }

    let license_fields =
        if let Some(checksum_value) = fields.last().and_then(|field| field.strip_prefix("ck=")) {
            let mut checksum = 0u32;
            for field in &fields[..fields.len() - 1] {
                checksum += field
                    .encode_utf16()
                    .map(|unit| u32::from(unit & 255))
                    .sum::<u32>();
            }
            if checksum_value.parse::<u32>().ok() != Some(checksum) {
                return Err(EncryptedKrpanoError::MissingKey);
            }
            &fields[..fields.len() - 1]
        } else {
            fields.as_slice()
        };

    for field in license_fields {
        if field.len() < 4 || !field.starts_with(key_tag) {
            continue;
        }
        let mut key = field[3..].to_string();
        if key.is_empty() {
            return Err(EncryptedKrpanoError::MissingKey);
        }
        // Pad to 128 characters (case 7 behavior)
        if key.len() < 128 {
            let original = key.clone();
            let mut original_chars = original.chars().cycle();
            while key.len() < 128 {
                key.push(
                    original_chars
                        .next()
                        .ok_or(EncryptedKrpanoError::MissingKey)?,
                );
            }
        }
        return Ok(key);
    }

    Err(EncryptedKrpanoError::MissingKey)
}

#[cfg(test)]
mod tests {
    use super::super::viewer;
    use super::*;
    use std::fs;
    use std::path::Path;

    fn load_fixture(fixture: &str) -> (Vec<u8>, String) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/krpano/encrypted")
            .join(fixture);
        let js_path = ["tour.js", "krpano.js"]
            .iter()
            .map(|name| root.join(name))
            .find(|path| path.exists())
            .unwrap();
        let js = fs::read(js_path).unwrap();
        let decoded = viewer::extract_decoded_viewer_js(&js).unwrap();
        let key = viewer::extract_key_from_viewer_js(&js).unwrap();
        (decoded, key)
    }

    #[test]
    fn derives_old_license_keys() {
        for fixture in [
            "old",
            "2013-06-05-B",
            "2013-08-09-B",
            "2015-08-04",
            "2017-09-21",
        ] {
            let (decoded, wrapper_key) = load_fixture(fixture);
            let ctx = derive_old_license_key(&decoded, &wrapper_key)
                .unwrap_or_else(|err| panic!("{fixture}: {err}"));
            if fixture.ends_with("-B") {
                assert!(
                    !ctx.default_key.is_empty(),
                    "{fixture}: empty old default key"
                );
                assert!(
                    !ctx.base64_alphabet.is_empty(),
                    "{fixture}: empty old Base64 alphabet"
                );
            }
            if fixture != "2013-08-09-B" {
                assert!(
                    ctx.protected_key
                        .as_ref()
                        .is_some_and(|key| key.len() >= 128),
                    "{fixture}: missing or short old protected key"
                );
            }
        }
    }
}
