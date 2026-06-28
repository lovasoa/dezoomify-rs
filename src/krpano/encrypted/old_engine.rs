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
        .or_else(|| {
            // Final fallback: some engines hardcode the alphabet in the source
            // (not in _[] rows). Search the decoded engine text for a
            // quoted string literal that looks like a 65+ char Base64 alphabet.
            find_base64_alphabet_in_source(decoded_engine)
        })
        .or_else(|| {
            // Some engines build the alphabet at runtime from several _[] rows
            // and string transforms (e.g. `w=_[N],w=w+(F(w)+_[M])` where F is
            // toLowerCase), rather than storing it as a single literal/row.
            // Parse and evaluate such constructions.
            find_constructed_alphabet(decoded_engine, &unpacked.rows)
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
    let b64_pos = decoded_engine.find("b64u8=function")?;

    // Try to find `_[N]` inside the helper functions called by b64u8.
    if let Some(body) = extract_function_body(decoded_engine, b64_pos + "b64u8=".len()) {
        let helper_names = extract_called_function_names(&body);
        for name in &helper_names {
            if let Some(idx) = find_row_ref_in_helper(decoded_engine, name) {
                return Some(idx);
            }
        }
    }

    // Fallback 1: search backward from b64u8 for any `_[N]` reference
    // (the old approach; works for some engines).
    let before_marker = &decoded_engine[..b64_pos];
    if let Some(idx) = extract_row_ref_before(before_marker) {
        return Some(idx);
    }

    // Fallback 2: search forward from b64u8 for `_[` in the function body
    let after_marker = &decoded_engine[b64_pos..];
    if let Some(end) = after_marker.find('}') {
        let body_range = &after_marker[..end + 1];
        if let Some(idx) = find_row_ref_in_text(body_range) {
            return Some(idx);
        }
    }

    None
}

/// Extract the function body text (between `{` and matching `}`) starting
/// at `open_brace_pos`.
fn extract_function_body(text: &str, after_function_keyword: usize) -> Option<String> {
    let rest = &text[after_function_keyword..];
    let open = rest.find('{')?;
    let mut depth = 1u32;
    let mut pos = open + 1;
    let bytes = rest.as_bytes();
    while pos < bytes.len() && depth > 0 {
        match bytes[pos] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        pos += 1;
    }
    if depth == 0 {
        Some(rest[open..pos].to_string())
    } else {
        None
    }
}

/// Extract function-call names from a JavaScript function body.
/// Looks for patterns like `g(a(d))` or `Td(Qd(d))` — captures the
/// function names that appear before `(`.
fn extract_called_function_names(body: &str) -> Vec<String> {
    let mut names = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' && i > 0 {
            // Walk backward to find the start of the function name
            let mut j = i - 1;
            while j > 0 && bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' {
                if j == 0 {
                    break;
                }
                j -= 1;
            }
            if j < i - 1 {
                let start = if bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' {
                    j
                } else {
                    j + 1
                };
                if start < i {
                    let name = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
                    // Filter out common non-function tokens
                    if !matches!(name, "" | "if" | "for" | "while" | "switch" | "return" | "function" | "typeof" | "var" | "let" | "const" | "new" | "d")
                        && name.len() >= 1
                    {
                        if !names.contains(&name.to_string()) {
                            names.push(name.to_string());
                        }
                    }
                }
            }
        }
        i += 1;
    }
    names
}

/// Search for a `_[N]` row reference inside a helper function's definition.
fn find_row_ref_in_helper(text: &str, helper_name: &str) -> Option<usize> {
    // Look for the helper function definition: `function <name>(` or
    // `<name>=function(`
    let pattern1 = format!("function {helper_name}(");
    let pattern2 = format!("{helper_name}=function(");

    for pattern in [&pattern1, &pattern2] {
        if let Some(pos) = text.find(pattern.as_str()) {
            // Extract a generous window around the function
            let start = pos.saturating_sub(20);
            let end = (pos + 2000).min(text.len());
            let window = &text[start..end];
            if let Some(idx) = find_row_ref_in_text(window) {
                return Some(idx);
            }
        }
    }
    None
}

/// Find a `_[N]` index reference in arbitrary text.  Searches for
/// the pattern `_[digits]` and parses the digits.
fn find_row_ref_in_text(text: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(pos) = text[search_from..].find("_[") {
        let abs_pos = search_from + pos + 2;
        let rest = &text[abs_pos..];
        if let Some(end) = rest.find(']') {
            let digits = &rest[..end];
            if digits.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(idx) = digits.parse::<usize>() {
                    return Some(idx);
                }
            }
        }
        search_from = abs_pos;
    }
    None
}

/// Find a `_[N]` reference by searching backward for `=_[
fn extract_row_ref_before(text: &str) -> Option<usize> {
    let row_ref_pos = text.rfind("=_[")? + 3;
    let digits_end = text[row_ref_pos..]
        .find(']')
        .map(|end| row_ref_pos + end)?;
    text[row_ref_pos..digits_end].parse().ok()
}

/// Search the decoded engine source text for a hardcoded Base64 alphabet
/// string literal (not stored in `_[]` rows or we.subdiv rows).
///
/// Some engines embed the alphabet directly in the JS source as a quoted
/// string, or construct it from character codes via `String.fromCharCode`.
/// This scans for both forms.
pub fn find_base64_alphabet_in_source(decoded_engine: &str) -> Option<String> {
    // First, search for plain quoted string literals.
    if let Some(result) = find_quoted_alphabet_strings(decoded_engine) {
        return Some(result);
    }
    // Second, search for alphabet constructed via String.fromCharCode(...).
    // Pattern: 65 comma-separated numbers.
    find_alphabet_from_charcodes(decoded_engine)
}

fn find_quoted_alphabet_strings(decoded_engine: &str) -> Option<String> {
    let bytes = decoded_engine.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let quote = match bytes[i] {
            b'"' | b'\'' => bytes[i],
            _ => {
                i += 1;
                continue;
            }
        };
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != quote {
            if bytes[i] == b'\\' {
                i += 2;
            } else {
                i += 1;
            }
        }
        let end = i;
        let len = end - start;
        // Base64 alphabets are ~65 chars (64 + padding). Accept 64..70.
        if (64..=70).contains(&len) {
            let candidate = &decoded_engine[start..end];
            if is_likely_base64_alphabet(candidate) {
                return Some(candidate.to_string());
            }
        }
        i += 1;
    }
    None
}

/// Search for an alphabet constructed from character codes, like
/// `String.fromCharCode(65,66,67,...)` or `[65,66,67,...]`.
fn find_alphabet_from_charcodes(decoded_engine: &str) -> Option<String> {
    // Pattern: `fromCharCode(` followed by 64+ comma-separated numbers
    let mut search_from = 0;
    while let Some(pos) = decoded_engine[search_from..].find("fromCharCode(") {
        let start = search_from + pos + "fromCharCode(".len();
        let rest = &decoded_engine[start..];
        // Find the closing )
        if let Some(close) = rest.find(')') {
            let args = &rest[..close];
            // Split by comma, parse numbers
            let nums: Vec<u32> = args
                .split(',')
                .filter_map(|s| s.trim().parse::<u32>().ok())
                .collect();
            if (64..=70).contains(&nums.len()) {
                // All nums should be printable ASCII (32-126)
                if nums.iter().all(|&n| (32..=126).contains(&n)) {
                    let alphabet: String = nums.iter().map(|&n| n as u8 as char).collect();
                    if is_likely_base64_alphabet(&alphabet) {
                        return Some(alphabet);
                    }
                }
            }
        }
        search_from = start;
    }
    None
}

/// Heuristic: does this string look like a Base64 alphabet?
/// A Base64 alphabet is a permutation of A-Za-z0-9+/ with '=' padding.
fn is_likely_base64_alphabet(s: &str) -> bool {
    if s.len() < 64 || s.len() > 70 {
        return false;
    }
    let mut seen = [false; 128];
    let mut unique_count = 0u32;
    for b in s.bytes() {
        if b >= 128 {
            return false;
        }
        if b != b'=' && !b.is_ascii_alphanumeric() && b != b'+' && b != b'/' {
            return false;
        }
        if !seen[b as usize] {
            seen[b as usize] = true;
            unique_count += 1;
        }
    }
    // The alphabet should have mostly unique characters (at least 60 unique).
    // The padding '=' character may appear at the end.
    unique_count >= 60
}

// ---------------------------------------------------------------------------
// Constructed-alphabet extraction
//
// Some old engines do not store the ClassicB Base64 alphabet as a single
// literal or `_[N]` row. Instead they build it at runtime from several
// `_[N]` rows and string transforms, e.g.:
//
//     var w=_[183], w=w+(F(w)+_[273]);      // F(a){return(""+a).toLowerCase()}
//
// which yields _[183] + lowercase(_[183]) + _[273].  The code below locates
// the Base64 decode function by its behavioral signature (indexOf + charAt
// + bit manipulation), follows the alphabet variable back to its
// assignment(s), and evaluates the construction expression against the
// unpacked `_[]` rows.  It is name-agnostic so it generalises to unseen
// minified builds.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Transform {
    Lowercase,
    Uppercase,
}

/// Find a Base64 alphabet that is constructed at runtime from `_[]` rows and
/// string transforms, rather than stored as a single literal/row.
fn find_constructed_alphabet(decoded_engine: &str, rows: &[String]) -> Option<String> {
    let decode_region = find_base64_decode_region(decoded_engine)?;
    let alphabet_var = find_alphabet_variable(decoded_engine, decode_region.clone())?;
    let transforms = detect_transform_functions(decoded_engine);
    let alphabet = evaluate_alphabet_construction(
        decoded_engine,
        &alphabet_var,
        rows,
        &transforms,
    )?;
    if is_likely_base64_alphabet(&alphabet) {
        Some(alphabet)
    } else {
        None
    }
}

/// Locate the Base64 decode function in the engine source by its behavioral
/// signature: a region containing `indexOf(`, `charAt`, and Base64 bit
/// manipulation (`<<2`, `>>4`, `&15`, `&3`, `<<6`). Returns the byte range
/// of a generous window around the first match.
fn find_base64_decode_region(src: &str) -> Option<std::ops::Range<usize>> {
    let bytes = src.as_bytes();
    let mut search = 0;
    while let Some(rel) = src[search..].find("indexOf(") {
        let abs = search + rel;
        search = abs + 7;
        let win_end = (abs + 400).min(src.len());
        let win = &src[abs..win_end];
        let has_charat = win.contains("charAt");
        let has_bitops = win.contains("<<2")
            || win.contains(">>4")
            || win.contains("<<6")
            || win.contains("&15)")
            || win.contains("&3)<<");
        if !has_charat || !has_bitops {
            continue;
        }
        // The decode function starts before this indexOf. Find the enclosing
        // function start by scanning back to `function` or `=function`.
        let back = &src[..abs];
        let fn_start = back
            .rfind("=function(")
            .map(|p| p + 1)
            .or_else(|| back.rfind("function "))
            .map(|p| p.saturating_sub(40));
        let start = fn_start.unwrap_or(abs.saturating_sub(400));
        return Some(start..win_end);
    }
    let _ = bytes; // (bytes retained for clarity; search uses str::find)
    None
}

/// Given the decode-function region, find the name of the closure variable
/// that holds the alphabet. The decode function looks like
/// `... <recv>=<alias>,... <recv>.indexOf(a.charAt(...))`; `<alias>` is either
/// the alphabet variable directly or a local bound to it. We follow a single
/// aliasing hop (local -> closure var).
fn find_alphabet_variable(src: &str, region: std::ops::Range<usize>) -> Option<String> {
    let region_text = &src[region.clone()];
    let idx = region_text.find(".indexOf(")?;
    // Walk back from `.indexOf` to capture the receiver identifier.
    let recv_end = region.start + idx;
    let recv = identifier_before(src, recv_end)?;
    if recv.is_empty() {
        return None;
    }
    // Find `<recv>=<value>` within the region (e.g. `var d=w`).
    let assign = find_local_assignment(region_text, &recv)?;
    let value = assign.trim();
    // If the value is a simple identifier, it is the alphabet variable.
    // If it is a row reference or expression, the alphabet is constructed
    // inline; return the receiver itself as the "variable" so the caller
    // evaluates the inline expression.
    if value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !value.bytes().next().is_some_and(|b| b.is_ascii_digit())
    {
        Some(value.to_string())
    } else {
        Some(recv)
    }
}

/// Build a map of single-argument function names that perform a case
/// transform, by scanning for definitions like
/// `function F(a){return(""+a).toLowerCase()}` and the `=function` variant.
fn detect_transform_functions(src: &str) -> std::collections::HashMap<String, Transform> {
    let mut map = std::collections::HashMap::new();
    for (needle, tfm) in [
        (".toLowerCase()", Transform::Lowercase),
        (".toUpperCase()", Transform::Uppercase),
    ] {
        let mut search = 0;
        while let Some(rel) = src[search..].find(needle) {
            let abs = search + rel;
            search = abs + needle.len();
            // Look back for `function <name>(<arg>){return` or `<name>=function(<arg>){return`.
            let back = &src[..abs];
            let body_start = back
                .rfind("{return")
                .or_else(|| back.rfind("{return \"\"+"))
                .or_else(|| back.rfind("return(\"\"+"));
            let Some(bs) = body_start else {
                continue;
            };
            let head = &src[..bs];
            // Try `function NAME(ARG)` then `NAME=function(ARG)`.
            if let Some(name) = last_function_decl_name(head, "function ") {
                map.insert(name, tfm);
            } else if let Some(name) = last_function_decl_name(head, "=function(") {
                map.insert(name, tfm);
            }
        }
    }
    map
}

/// Extract the function name immediately preceding a `function ` or
/// `=function(` marker, scanning backward from the end of `head`.
fn last_function_decl_name(head: &str, marker: &str) -> Option<String> {
    let pos = head.rfind(marker)?;
    let after = if marker == "function " {
        pos + marker.len()
    } else {
        // `NAME=function(` -> NAME is before `=`.
        let eq = pos; // pos points at '=' in '=function('
        let name_end = eq;
        let name = identifier_before(head, name_end)?;
        if name.is_empty() {
            return None;
        }
        return Some(name);
    };
    // `function NAME(...` -> read identifier after marker.
    let bytes = head.as_bytes();
    let mut i = after;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    if i > after {
        Some(head[after..i].to_string())
    } else {
        None
    }
}

/// Collect all assignments to `var` (in source order) and evaluate them,
/// returning the final value. Assignments may be comma-separated within a
/// single `var` declaration or separate statements.
fn evaluate_alphabet_construction(
    src: &str,
    var: &str,
    rows: &[String],
    transforms: &std::collections::HashMap<String, Transform>,
) -> Option<String> {
    let mut scope: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut last_value: Option<String> = None;
    for rhs in collect_assignments(src, var) {
        if let Some(val) = eval_expr(&rhs, rows, transforms, &scope) {
            scope.insert(var.to_string(), val.clone());
            last_value = Some(val);
        }
    }
    last_value
}

/// Find the RHS expressions of all `<var>=...` assignments in source order.
/// Matches word boundaries so `w=` does not match `sw=`. Skips `==`.
fn collect_assignments(src: &str, var: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let v = var.as_bytes();
    let mut i = 0;
    while i + v.len() + 1 <= bytes.len() {
        // Match `var` at a word boundary.
        if &bytes[i..i + v.len()] == v {
            let before_ok = i == 0
                || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'
                    || bytes[i - 1] == b'.');
            let after = i + v.len();
            if before_ok && after < bytes.len() && bytes[after] == b'=' && bytes.get(after + 1) != Some(&b'=')
            {
                // Skip an optional leading `var ` keyword if present just
                // before; not required for RHS extraction.
                let rhs_start = after + 1;
                if let Some(rhs) = read_rhs(&src[rhs_start.min(src.len())..]) {
                    out.push(rhs);
                    i = rhs_start + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// Read an RHS expression up to the next `,` or `;` at brace/paren depth 0,
/// respecting string literals.
fn read_rhs(rest: &str) -> Option<String> {
    let bytes = rest.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                let q = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth -= 1,
            b',' | b';' if depth <= 0 => return Some(rest[..i].trim().to_string()),
            _ => {}
        }
        i += 1;
    }
    Some(rest.trim().to_string())
}

/// Evaluate a restricted JS string expression.
///
/// Grammar (subset):
///   expr  := term ('+' term)*
///   term  := factor ( '.' method '(' ')' )*
///   factor:= '_[' NUM ']' | STRING | IDENT '(' expr ')' | IDENT | '(' expr ')'
///   method:= 'toLowerCase' | 'toUpperCase'
fn eval_expr(
    expr: &str,
    rows: &[String],
    transforms: &std::collections::HashMap<String, Transform>,
    scope: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let mut p = Parser::new(expr.as_bytes());
    let val = p.parse_expr(rows, transforms, scope)?;
    p.skip_ws();
    if p.pos == p.bytes.len() {
        Some(val)
    } else {
        None
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.bytes.get(self.pos).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        self.skip_ws();
        if self.bytes.get(self.pos) == Some(&c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn parse_expr(
        &mut self,
        rows: &[String],
        transforms: &std::collections::HashMap<String, Transform>,
        scope: &std::collections::HashMap<String, String>,
    ) -> Option<String> {
        let mut acc = self.parse_term(rows, transforms, scope)?;
        loop {
            if self.eat(b'+') {
                let rhs = self.parse_term(rows, transforms, scope)?;
                acc.push_str(&rhs);
            } else {
                break;
            }
        }
        Some(acc)
    }
    fn parse_term(
        &mut self,
        rows: &[String],
        transforms: &std::collections::HashMap<String, Transform>,
        scope: &std::collections::HashMap<String, String>,
    ) -> Option<String> {
        let mut val = self.parse_factor(rows, transforms, scope)?;
        loop {
            self.skip_ws();
            if self.eat(b'.') {
                let name = self.read_ident();
                self.skip_ws();
                if self.eat(b'(') {
                    self.skip_ws();
                    self.eat(b')');
                    val = match name.as_str() {
                        "toLowerCase" => val.to_lowercase(),
                        "toUpperCase" => val.to_uppercase(),
                        _ => return None,
                    };
                } else {
                    return None;
                }
            } else {
                break;
            }
        }
        Some(val)
    }
    fn parse_factor(
        &mut self,
        rows: &[String],
        transforms: &std::collections::HashMap<String, Transform>,
        scope: &std::collections::HashMap<String, String>,
    ) -> Option<String> {
        self.skip_ws();
        match self.peek()? {
            b'(' => {
                self.pos += 1;
                let v = self.parse_expr(rows, transforms, scope)?;
                self.eat(b')');
                Some(v)
            }
            b'"' | b'\'' => Some(self.read_string()?),
            b'_' => {
                // _[N]
                self.pos += 1;
                self.eat(b'[');
                let n = self.read_number()?;
                self.eat(b']');
                rows.get(n).cloned()
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let name = self.read_ident();
                self.skip_ws();
                if self.eat(b'(') {
                    // Function call: FNAME(expr)
                    let arg = self.parse_expr(rows, transforms, scope)?;
                    self.eat(b')');
                    match transforms.get(&name).copied() {
                        Some(Transform::Lowercase) => Some(arg.to_lowercase()),
                        Some(Transform::Uppercase) => Some(arg.to_uppercase()),
                        None => None,
                    }
                } else {
                    scope.get(&name).cloned()
                }
            }
            _ => None,
        }
    }
    fn read_ident(&mut self) -> String {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.bytes.len()
            && (self.bytes[self.pos].is_ascii_alphanumeric() || self.bytes[self.pos] == b'_')
        {
            self.pos += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.pos])
            .unwrap_or("")
            .to_string()
    }
    fn read_number(&mut self) -> Option<usize> {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.pos])
            .ok()?
            .parse()
            .ok()
    }
    fn read_string(&mut self) -> Option<String> {
        let q = self.bytes[self.pos];
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.bytes.len() && self.bytes[self.pos] != q {
            if self.bytes[self.pos] == b'\\' {
                self.pos += 1;
            }
            self.pos += 1;
        }
        let s = std::str::from_utf8(&self.bytes[start..self.pos]).ok()?.to_string();
        if self.pos < self.bytes.len() {
            self.pos += 1; // closing quote
        }
        Some(s)
    }
}

/// Return the identifier (letters/digits/underscore) ending immediately
/// before `end` (exclusive), scanning backward and skipping whitespace.
fn identifier_before(src: &str, end: usize) -> Option<String> {
    let bytes = src.as_bytes();
    let mut i = end;
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    let name_end = i;
    while i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
        i -= 1;
    }
    if i < name_end {
        Some(src[i..name_end].to_string())
    } else {
        None
    }
}

/// Find `<name>=<value>` within `text`, returning the value portion. Matches
/// a word boundary before `<name>` and a single `=` (not `==`).
fn find_local_assignment(text: &str, name: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let n = name.as_bytes();
    let mut search = 0;
    while search + n.len() + 1 <= bytes.len() {
        if let Some(rel) = text[search..].find(name) {
            let abs = search + rel;
            search = abs + n.len();
            let before_ok = abs == 0
                || !(bytes[abs - 1].is_ascii_alphanumeric() || bytes[abs - 1] == b'_'
                    || bytes[abs - 1] == b'.');
            let after = abs + n.len();
            if !before_ok || after >= bytes.len() || bytes[after] != b'=' || bytes.get(after + 1) == Some(&b'=')
            {
                continue;
            }
            return read_rhs(&text[after + 1..]);
        } else {
            break;
        }
    }
    None
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
