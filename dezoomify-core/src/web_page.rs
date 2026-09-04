//! Generic HTML page parsing helpers shared by the site-specific dezoomers.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

static META_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<meta\b[^>]*>").expect("constant meta tag pattern"));
static TITLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<title\b[^>]*>([^<]*)</title>").expect("constant title pattern")
});
static ATTRIBUTE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)([A-Za-z_:][A-Za-z0-9_.:-]*)\s*=\s*["']([^"']*)["']"#)
        .expect("constant attribute pattern")
});

/// Best-effort human-readable title of an HTML page.
///
/// Prefers Open Graph and Twitter Card metadata over the plain `<title>`
/// element, decodes HTML entities, and returns `None` when the page declares
/// nothing meaningful. Dezoomers use it to name images after the page that
/// embeds them instead of inventing a generic name.
#[must_use]
pub fn page_title(page: &str) -> Option<String> {
    META_RE
        .captures_iter(page)
        .find_map(|captures| {
            let tag = captures.get(0)?.as_str();
            let key = attribute(tag, "property").or_else(|| attribute(tag, "name"))?;
            let is_title =
                key.eq_ignore_ascii_case("og:title") || key.eq_ignore_ascii_case("twitter:title");
            if !is_title {
                return None;
            }
            let title = decode_html_entities(attribute(tag, "content")?);
            let title = title.trim();
            (!title.is_empty()).then(|| title.to_owned())
        })
        .or_else(|| {
            TITLE_RE.captures(page).and_then(|captures| {
                let title = decode_html_entities(captures.get(1)?.as_str());
                let title = title.trim();
                (!title.is_empty()).then(|| title.to_owned())
            })
        })
}

fn attribute<'a>(tag: &'a str, wanted: &str) -> Option<&'a str> {
    ATTRIBUTE_RE.captures_iter(tag).find_map(|captures| {
        (captures.get(1)?.as_str().eq_ignore_ascii_case(wanted))
            .then(|| captures.get(2).expect("attribute value capture").as_str())
    })
}

/// Replace the HTML entities found in `text` by the characters they encode.
///
/// Entities without a known expansion are kept verbatim.
#[must_use]
pub fn decode_html_entities(text: &str) -> String {
    static HTML_ENTITIES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
        HashMap::from([
            ("amp", "&"),
            ("lt", "<"),
            ("gt", ">"),
            ("quot", "\""),
            ("apos", "'"),
            ("nbsp", "\u{00A0}"),
            ("iexcl", "¡"),
            ("cent", "¢"),
            ("pound", "£"),
            ("curren", "¤"),
            ("yen", "¥"),
            ("brvbar", "¦"),
            ("sect", "§"),
            ("uml", "¨"),
            ("copy", "©"),
            ("ordf", "ª"),
            ("laquo", "«"),
            ("not", "¬"),
            ("shy", "\u{00AD}"),
            ("reg", "®"),
            ("macr", "¯"),
            ("deg", "°"),
        ])
    });

    static ENTITY_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"&([a-zA-Z][a-zA-Z0-9]*|#[0-9]+|#x[0-9a-fA-F]+);")
            .expect("constant HTML entity pattern")
    });

    ENTITY_RE
        .replace_all(text, |captures: &regex::Captures| {
            let entity = &captures[1];

            if let Some(&replacement) = HTML_ENTITIES.get(entity) {
                return replacement.to_owned();
            }

            // Handle numeric entities like &#123; or &#x1A;
            if let Some(stripped) = entity.strip_prefix('#') {
                let code = if let Some(hex_stripped) = stripped.strip_prefix("x") {
                    u32::from_str_radix(hex_stripped, 16).ok()
                } else {
                    stripped.parse::<u32>().ok()
                };
                if let Some(character) = code.and_then(char::from_u32) {
                    return character.to_string();
                }
            }

            format!("&{entity};")
        })
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_title_prefers_open_graph_metadata() {
        assert_eq!(
            page_title(concat!(
                "<meta property=\"og:image\" content=\"https://fixtures.test/a.jpg\">",
                "<meta property=\"og:title\" content=\"Негатив: У фонтанов\">"
            )),
            Some("Негатив: У фонтанов".to_owned())
        );
        assert_eq!(
            page_title("<meta name=\"twitter:title\" content=\"Fallback &amp; Co\">"),
            Some("Fallback & Co".to_owned())
        );
        assert_eq!(
            page_title("<title>Plain page title</title>"),
            Some("Plain page title".to_owned())
        );
        assert_eq!(page_title("<title>   </title><meta name=\"x\">"), None);
    }

    #[test]
    fn html_entities_are_decoded() {
        assert_eq!(decode_html_entities("a &amp; b"), "a & b");
        assert_eq!(decode_html_entities("&#39;"), "'");
        assert_eq!(decode_html_entities("&#x263A;"), "☺");
        assert_eq!(decode_html_entities("&unknown;"), "&unknown;");
    }
}
