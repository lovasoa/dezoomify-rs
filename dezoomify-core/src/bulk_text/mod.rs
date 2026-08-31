//! Pure discovery for text files containing deferred image URLs.

use crate::core::{
    CatalogEntry, DeferredImage, DezoomerSpec, DiscoveryError, DiscoveryMatch, ImageCatalog,
    StableId,
};

pub const SPEC: DezoomerSpec =
    DezoomerSpec::new("bulk_text", &[DiscoveryMatch::Any.extract(catalog)])
        .recognizing(is_bulk_file, "not a bulk URL-list file");

fn catalog(uri: &str, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let text = std::str::from_utf8(bytes).map_err(|error| {
        DiscoveryError::Session(format!("failed to parse bulk list as UTF-8: {error}"))
    })?;
    let images = parse_text_urls_with_base(text, uri)?;
    if images.is_empty() {
        return Err(DiscoveryError::Session(
            "no valid URLs found in text file".into(),
        ));
    }
    Ok(ImageCatalog::new(images.into_iter().enumerate().map(
        |(index, image)| {
            CatalogEntry::Deferred(DeferredImage {
                id: StableId::new(format!("bulk:{index}")),
                uri: image.uri,
                title: image.title,
                warnings: Vec::new(),
            })
        },
    )))
}

fn is_bulk_file(uri: &str) -> bool {
    (uri.rsplit('.').next().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("txt") || extension.eq_ignore_ascii_case("urls")
    }) || uri.contains("bulk")
        || uri.contains("list"))
        && !uri.contains("{{")
        && !uri.contains("}}")
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ListedImage {
    uri: String,
    title: Option<String>,
}

#[cfg(test)]
fn parse_text_urls(content: &str) -> Result<Vec<ListedImage>, DiscoveryError> {
    parse_text_urls_with_base(content, "")
}

fn parse_text_urls_with_base(
    content: &str,
    base_uri: &str,
) -> Result<Vec<ListedImage>, DiscoveryError> {
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.trim();
            (!line.is_empty() && !line.starts_with('#')).then(|| (index + 1, line))
        })
        .map(|(line_number, line)| {
            let mut parts = line.splitn(2, char::is_whitespace);
            let raw_uri = parts.next().unwrap_or_default();
            validate_uri(raw_uri, line_number)?;
            let uri = normalize_uri(raw_uri, base_uri);
            let title = parts
                .next()
                .filter(|title| !title.is_empty())
                .map(str::to_owned);
            Ok(ListedImage { uri, title })
        })
        .collect()
}

fn normalize_uri(uri: &str, base_uri: &str) -> String {
    if url::Url::parse(uri).is_ok()
        || uri.contains("{{X}}")
        || uri.contains("{{Y}}")
        || uri.starts_with(['/', '.', '\\'])
        || uri.contains(['/', '\\'])
        || (uri.len() >= 2 && uri.as_bytes()[1] == b':' && uri.as_bytes()[0].is_ascii_alphabetic())
    {
        return uri.to_owned();
    }
    if base_uri.is_empty() {
        return uri.to_owned();
    }
    let base_path = if let Ok(url) = url::Url::parse(base_uri) {
        url.path().to_owned()
    } else {
        base_uri.to_owned()
    };
    if let Some(slash) = base_path.rfind('/') {
        let dir = &base_path[..=slash];
        if let Ok(url) = url::Url::parse(base_uri) {
            if let Ok(joined) = url.join(uri) {
                return joined.to_string();
            }
            return format!("{dir}{uri}");
        }
        return format!("{dir}{uri}");
    }
    uri.to_owned()
}

fn validate_uri(input: &str, line: usize) -> Result<(), DiscoveryError> {
    if url::Url::parse(input).is_ok()
        || input.contains("{{X}}")
        || input.contains("{{Y}}")
        || input.starts_with(['/', '.', '\\'])
        || input.contains(['/', '\\'])
        || (input.len() >= 2
            && input.as_bytes()[1] == b':'
            && input.as_bytes()[0].is_ascii_alphabetic())
        || (input.contains('.') && !input.contains(' ') && !input.contains("://"))
    {
        return Ok(());
    }
    Err(DiscoveryError::Session(format!(
        "on line {line}: '{input}' is not a valid URL or file path"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(uri: &str, content: &str) -> Result<ImageCatalog, DiscoveryError> {
        catalog(uri, content.as_bytes())
    }

    #[test]
    fn parses_urls_titles_paths_and_invalid_content() {
        assert!(parse_text_urls("").unwrap().is_empty());
        assert!(
            parse_text_urls("# comment\n\n   \n# another")
                .unwrap()
                .is_empty()
        );
        let images = parse_text_urls(
            "# comment\nhttps://example.test/a.jpg A title\n./local.dzi\nhttps://example.org/manifest.json",
        )
        .unwrap();
        assert_eq!(
            images,
            vec![
                ListedImage {
                    uri: "https://example.test/a.jpg".into(),
                    title: Some("A title".into())
                },
                ListedImage {
                    uri: "./local.dzi".into(),
                    title: None
                },
                ListedImage {
                    uri: "https://example.org/manifest.json".into(),
                    title: None
                },
            ]
        );
        let urls = parse_text_urls(
            "http://example.com/image1.jpg My Custom Title\nhttps://example.org/manifest.json Another Title",
        )
        .unwrap();
        assert_eq!(urls[0].title.as_deref(), Some("My Custom Title"));
        assert_eq!(urls[1].title.as_deref(), Some("Another Title"));
        let error = parse_text_urls("not_a_valid_url").unwrap_err().to_string();
        assert!(error.contains("line 1") && error.contains("not_a_valid_url"));
    }

    #[test]
    fn session_completes_deferred_entries_and_rejects_bad_lists() {
        let catalog = complete(
            "file://test.txt",
            "http://example.com/image1.jpg\nhttps://example.org/manifest.json",
        )
        .unwrap();
        assert_eq!(catalog.len(), 2);
        let entries = catalog.into_entries();
        assert!(
            matches!(entries[0], CatalogEntry::Deferred(DeferredImage { ref uri, title: None, .. }) if uri == "http://example.com/image1.jpg")
        );
        assert!(
            matches!(entries[1], CatalogEntry::Deferred(DeferredImage { ref uri, .. }) if uri == "https://example.org/manifest.json")
        );

        let error = complete("file://empty.txt", "# Only comments\n\n# Nothing else")
            .unwrap_err()
            .to_string();
        assert!(error.contains("No valid URLs found") || error.contains("no valid URLs found"));
        let error = complete("file://invalid.txt", "not_a_valid_url")
            .unwrap_err()
            .to_string();
        assert!(error.contains("line 1") && error.contains("not_a_valid_url"));
    }
}
