//! Pure discovery for text files containing deferred image URLs.

use crate::core::discovery::DiscoveryEvent;
use crate::core::{
    CatalogEntry, DeferredImage, DiscoveryDiagnostic, DiscoveryError, DiscoveryInput,
    DiscoveryProgram, DiscoverySession, DiscoveryStep, ImageCatalog, ResourceOutcome, ResourceRequest, StableId,
};

/// Text-list discovery program.
#[derive(Default)]
pub struct BulkTextDezoomer;

impl DiscoveryProgram for BulkTextDezoomer {
    fn start(&self, input: &DiscoveryInput) -> Box<dyn DiscoverySession> {
        Box::new(BulkSession {
            uri: input.uri.clone(),
            requested: false,
        })
    }
}

struct BulkSession {
    uri: String,
    requested: bool,
}

impl DiscoverySession for BulkSession {
    fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
        match event {
            DiscoveryEvent::Start if !is_bulk_file(&self.uri) => Ok(DiscoveryStep::Reject(
                DiscoveryDiagnostic::from("not a bulk URL-list file"),
            )),
            DiscoveryEvent::Start if !self.requested => {
                self.requested = true;
                Ok(DiscoveryStep::Need(ResourceRequest::new(
                    self.uri.clone(),
                )))
            }
            DiscoveryEvent::Resource(ResourceOutcome::Response(response)) => {
                let text = std::str::from_utf8(&response.bytes).map_err(|error| {
                    DiscoveryError::Session(format!("failed to parse bulk list as UTF-8: {error}"))
                })?;
                let images = parse_text_urls_with_base(text, &self.uri)?;
                if images.is_empty() {
                    return Err(DiscoveryError::Session(
                        "no valid URLs found in text file".into(),
                    ));
                }
                Ok(DiscoveryStep::Complete(ImageCatalog::new(
                    images.into_iter().enumerate().map(|(index, image)| {
                        CatalogEntry::Deferred(DeferredImage {
                            id: StableId::new(format!("bulk:{index}")),
                            uri: image.uri,
                            title: Some(image.title),
                            warnings: Vec::new(),
                        })
                    }),
                )))
            }
            DiscoveryEvent::Resource(ResourceOutcome::Failure(failure)) => {
                Err(DiscoveryError::Session(failure.message.clone()))
            }
            DiscoveryEvent::Start => {
                Err(DiscoveryError::Session("bulk session started twice".into()))
            }
        }
    }
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
    title: String,
}

#[cfg(test)]
fn parse_text_urls(content: &str) -> Result<Vec<ListedImage>, DiscoveryError> {
    parse_text_urls_with_base(content, "")
}

fn parse_text_urls_with_base(content: &str, base_uri: &str) -> Result<Vec<ListedImage>, DiscoveryError> {
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
                .map_or_else(|| title_from_uri(&uri, line_number), str::to_owned);
            Ok(ListedImage {
                uri,
                title,
            })
        })
        .collect()
}

fn normalize_uri(uri: &str, base_uri: &str) -> String {
    if url::Url::parse(uri).is_ok()
        || uri.contains("{{X}}")
        || uri.contains("{{Y}}")
        || uri.starts_with(['/', '.', '\\'])
        || uri.contains(['/', '\\'])
        || (uri.len() >= 2
            && uri.as_bytes()[1] == b':'
            && uri.as_bytes()[0].is_ascii_alphabetic())
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

fn title_from_uri(uri: &str, line: usize) -> String {
    url::Url::parse(uri)
        .ok()
        .and_then(|url| {
            url.path_segments()?
                .rfind(|segment| !segment.is_empty())
                .map(str::to_owned)
        })
        .map(|name| {
            name.rsplit_once('.')
                .map_or(name.clone(), |(stem, _)| stem.to_owned())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("URL_{line}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::discovery::{RequestId, ResourceOutcome, ResourceResponse};

    fn response(content: &str) -> ResourceOutcome {
        ResourceOutcome::Response(ResourceResponse {
            id: RequestId(0),
            bytes: content.as_bytes().to_vec(),
        })
    }

    fn complete(uri: &str, content: &str) -> Result<ImageCatalog, DiscoveryError> {
        let mut session = BulkTextDezoomer.start(&DiscoveryInput::from(uri));
        assert!(matches!(
            session.advance(DiscoveryEvent::Start)?,
            DiscoveryStep::Need(_)
        ));
        match session.advance(DiscoveryEvent::Resource(&response(content)))? {
            DiscoveryStep::Complete(catalog) => Ok(catalog),
            _ => panic!("bulk discovery did not complete"),
        }
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
                    title: "A title".into()
                },
                ListedImage {
                    uri: "./local.dzi".into(),
                    title: "URL_3".into()
                },
                ListedImage {
                    uri: "https://example.org/manifest.json".into(),
                    title: "manifest".into()
                },
            ]
        );
        let urls = parse_text_urls(
            "http://example.com/image1.jpg My Custom Title\nhttps://example.org/manifest.json Another Title",
        )
        .unwrap();
        assert_eq!(urls[0].title, "My Custom Title");
        assert_eq!(urls[1].title, "Another Title");
        let error = parse_text_urls("not_a_valid_url").unwrap_err().to_string();
        assert!(error.contains("line 1") && error.contains("not_a_valid_url"));
        assert_eq!(title_from_uri("http://example.com/image.jpg", 1), "image");
        assert_eq!(
            title_from_uri("https://example.org/path/manifest.json", 2),
            "manifest"
        );
        assert_eq!(title_from_uri("http://example.com/", 3), "URL_3");
        assert_eq!(title_from_uri("not_a_url", 4), "URL_4");
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
            matches!(entries[0], CatalogEntry::Deferred(DeferredImage { ref uri, ref title, .. }) if uri == "http://example.com/image1.jpg" && title.as_deref() == Some("image1"))
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
