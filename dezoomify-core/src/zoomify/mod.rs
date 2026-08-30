//! Pure discovery for Zoomify viewers and `ImageProperties.xml` pyramids.

use std::sync::{Arc, LazyLock};

use image_properties::ImageProperties;
use regex::Regex;

use crate::Vec2d;
use crate::core::discovery::DiscoveryEvent;
use crate::core::{
    CatalogEntry, Dezoomer, DezoomerSpec, DiscoveryDiagnostic, DiscoveryError, DiscoveryInput,
    DiscoveryStep, Grid, ImageCatalog, ImageDescriptor, LevelDescriptor, Request, ResourceOutcome,
    ResourceRequest, StableId, resolve_relative,
};

mod image_properties;

pub const SPEC: DezoomerSpec = DezoomerSpec::stateful("zoomify", start).preferring(is_zoomify_url);

static SHOW_IMAGE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\bZ\s*\.\s*)?\bshowImage\s*\(").expect("constant Zoomify showImage pattern")
});

static FLASH_IMAGE_PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bzoomifyImagePath\s*=\s*([^'"&]*)(?:['"&])"#)
        .expect("constant Zoomify FlashVars pattern")
});

static HTML_BASE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<base\s+[^>]*\bhref\s*=\s*["']([^"']*)"#)
        .expect("constant HTML base pattern")
});

static TILE_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|/)TileGroup\d+/\d+-\d+-\d+\.jpe?g(?:[?#].*)?$")
        .expect("constant Zoomify tile URL pattern")
});

fn is_tile_url(uri: &str) -> bool {
    TILE_URL_RE.is_match(uri)
}

fn is_zoomify_url(uri: &str) -> bool {
    uri.contains("/ImageProperties.xml") || is_tile_url(uri)
}

struct Zoomify {
    input_uri: String,
    state: SessionState,
}

enum SessionState {
    Initial,
    WaitingPage,
    WaitingMetadata { metadata_uri: String },
    Complete,
}

fn start(input: &DiscoveryInput) -> Box<dyn Dezoomer> {
    Box::new(Zoomify {
        input_uri: input.uri.clone(),
        state: SessionState::Initial,
    })
}

impl Dezoomer for Zoomify {
    fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
        match event {
            DiscoveryEvent::Start if matches!(self.state, SessionState::Initial) => {
                if is_zoomify_url(&self.input_uri) {
                    let request = metadata_request(&self.input_uri)?;
                    let metadata_uri = request.request.uri.clone();
                    self.state = SessionState::WaitingMetadata { metadata_uri };
                    Ok(DiscoveryStep::Need(request))
                } else {
                    self.state = SessionState::WaitingPage;
                    Ok(DiscoveryStep::Need(ResourceRequest::new(
                        self.input_uri.clone(),
                    )))
                }
            }
            DiscoveryEvent::Start => Err(DiscoveryError::Session(
                "Zoomify session started twice".into(),
            )),
            DiscoveryEvent::Resource(ResourceOutcome::Failure(failure)) => {
                self.state = SessionState::Complete;
                Err(DiscoveryError::Session(format!(
                    "failed to download Zoomify resource: {}",
                    failure.message
                )))
            }
            DiscoveryEvent::Resource(ResourceOutcome::Response(response)) => {
                self.handle_response(&response.bytes)
            }
        }
    }
}

impl Zoomify {
    fn handle_response(&mut self, contents: &[u8]) -> Result<DiscoveryStep, DiscoveryError> {
        let state = std::mem::replace(&mut self.state, SessionState::Complete);
        match state {
            SessionState::WaitingPage => {
                if let Ok(catalog) = load_catalog(&self.input_uri, contents) {
                    return Ok(DiscoveryStep::Complete(catalog));
                }

                let html = String::from_utf8_lossy(contents);
                let Some(image_path) = extract_image_path(&html) else {
                    return Ok(DiscoveryStep::Reject(DiscoveryDiagnostic::from(
                        "not a Zoomify viewer page, metadata, or tile URL",
                    )));
                };
                let page_base_uri = extract_html_base(&html).map_or_else(
                    || self.input_uri.clone(),
                    |base| resolve_relative(&self.input_uri, &base),
                );
                let image_uri = resolve_relative(&page_base_uri, &image_path);
                let metadata_uri = append_path_component(&image_uri, "ImageProperties.xml");
                self.state = SessionState::WaitingMetadata {
                    metadata_uri: metadata_uri.clone(),
                };
                Ok(DiscoveryStep::Need(ResourceRequest::new(metadata_uri)))
            }
            SessionState::WaitingMetadata { metadata_uri } => Ok(DiscoveryStep::Complete(
                load_catalog(&metadata_uri, contents)?,
            )),
            SessionState::Initial => Err(DiscoveryError::Session(
                "Zoomify session received a resource before it started".into(),
            )),
            SessionState::Complete => Err(DiscoveryError::Session(
                "Zoomify session has already completed".into(),
            )),
        }
    }
}

fn append_path_component(uri: &str, component: &str) -> String {
    let suffix_start = uri.find(['?', '#']).unwrap_or(uri.len());
    let (path, suffix) = uri.split_at(suffix_start);
    format!("{}/{component}{suffix}", path.trim_end_matches('/'))
}

fn extract_image_path(html: &str) -> Option<String> {
    let show_image = SHOW_IMAGE_RE.find_iter(html).find_map(|marker| {
        second_javascript_string(&html[marker.end()..]).map(|path| (marker.start(), path))
    });
    let flash = FLASH_IMAGE_PATH_RE
        .captures_iter(html)
        .find_map(|captures| {
            Some((
                captures.get(0)?.start(),
                captures.get(1)?.as_str().replace("&amp;", "&"),
            ))
        });
    [show_image, flash]
        .into_iter()
        .flatten()
        .min_by_key(|(offset, _)| *offset)
        .map(|(_, path)| path)
}

fn extract_html_base(html: &str) -> Option<String> {
    HTML_BASE_RE
        .captures(html)
        .and_then(|captures| captures.get(1))
        .map(|base| base.as_str().replace("&amp;", "&"))
}

fn second_javascript_string(arguments: &str) -> Option<String> {
    let remaining = if let Some((remaining, _)) = javascript_string(arguments) {
        remaining.trim_start().strip_prefix(',')?
    } else {
        arguments.split_once(',')?.1
    };
    javascript_string(remaining).map(|(_, value)| value.replace("&amp;", "&"))
}

fn javascript_string(input: &str) -> Option<(&str, String)> {
    let input = input.trim_start();
    let quote = input.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }

    let mut value = String::new();
    let mut escaped = false;
    for (offset, character) in input[quote.len_utf8()..].char_indices() {
        if escaped {
            value.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == quote {
            let end = quote.len_utf8() + offset + character.len_utf8();
            return Some((&input[end..], value));
        } else {
            value.push(character);
        }
    }
    None
}

#[allow(clippy::unnecessary_wraps)]
fn metadata_request(input: &str) -> Result<ResourceRequest, DiscoveryError> {
    let uri = TILE_URL_RE.find(input).map_or_else(
        || input.to_owned(),
        |tile| {
            let prefix_end = if input.as_bytes()[tile.start()] == b'/' {
                tile.start() + 1
            } else {
                tile.start()
            };
            let prefix = &input[..prefix_end];
            let metadata = if prefix.is_empty() || prefix.ends_with('/') {
                format!("{prefix}ImageProperties.xml")
            } else {
                format!("{prefix}/ImageProperties.xml")
            };
            let suffix = tile
                .as_str()
                .find(['?', '#'])
                .map_or("", |index| &tile.as_str()[index..]);
            format!("{metadata}{suffix}")
        },
    );
    Ok(ResourceRequest::new(uri))
}

fn load_catalog(url: &str, contents: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let properties: ImageProperties = serde_xml_rs::from_reader(contents).map_err(|error| {
        DiscoveryError::Session(format!("unable to parse Zoomify XML: {error}"))
    })?;
    if properties.width == 0 || properties.height == 0 || properties.tile_size == 0 {
        return Err(DiscoveryError::Session(
            "Zoomify XML must declare positive WIDTH, HEIGHT, and TILESIZE values".into(),
        ));
    }
    let base_url: Arc<str> = url
        .split("/ImageProperties.xml")
        .next()
        .unwrap_or(url)
        .into();
    let base_name = base_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("Zoomify");
    let full_resolution_only = properties.is_full_resolution_only();
    let (level_info, warnings) = properties.levels_with_warnings();
    let levels = level_info
        .into_iter()
        .enumerate()
        .map(|(index, info)| {
            let size = info.size;
            let tile_size = info.tile_size;
            let base_url = Arc::clone(&base_url);
            let source = Grid::with_requests(
                format!("zoomify:{index}").into(),
                size,
                tile_size,
                Vec2d::default(),
                move |tile| {
                    let cell: Vec2d = tile.coord.into();
                    // Some producers declare only the full-resolution tile
                    // count and consequently store every level in TileGroup0.
                    let tile_group = if full_resolution_only {
                        0
                    } else {
                        (u64::from(info.tiles_before) + tile.row_major_ordinal) / 256
                    };
                    Request::new(format!(
                        "{base_url}/TileGroup{tile_group}/{index}-{}-{}.jpg",
                        cell.x, cell.y
                    ))
                },
            )
            .map_err(|error| DiscoveryError::Session(format!("invalid Zoomify grid: {error}")))?;
            Ok(LevelDescriptor::new(source).with_title(Some(format!(
                "{base_name} Zoomify level {index} ({: >5}×{: >5} pixels)",
                size.x, size.y,
            ))))
        })
        .collect::<Result<Vec<_>, DiscoveryError>>()?;
    let title = base_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("zoomify:image"),
        title,
        format: StableId::new("zoomify"),
        levels,
        warnings,
    })]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::TileSource;

    #[test]
    fn tile_urls_request_sibling_metadata() {
        let mut registry = crate::core::Registry::new();
        registry.register(SPEC);
        let mut operation =
            registry.start("https://example.com/images/book/TileGroup0/3-0-0.jpg?token=secret");
        let need = operation.missing_resources().unwrap().pop().unwrap();
        assert_eq!(
            need.request.uri,
            "https://example.com/images/book/ImageProperties.xml?token=secret"
        );
        operation
            .provide(crate::core::ResourceResponse {
                id: need.id,
                bytes: br#"<IMAGE_PROPERTIES WIDTH="512" HEIGHT="256" NUMTILES="2" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#.to_vec(),
            })
            .unwrap();
        let catalog = operation.finish().unwrap();
        let CatalogEntry::Ready(image) = &catalog.entries()[0] else {
            panic!("Zoomify metadata must produce a ready image")
        };
        let TileSource::Grid(plan) = &image.levels[0].source else {
            panic!("Zoomify levels must be grids")
        };
        assert_eq!(
            plan.tiles_row_major().next().unwrap().unwrap().request.uri,
            "https://example.com/images/book/TileGroup0/0-0-0.jpg"
        );
    }

    #[test]
    fn viewer_pages_respect_html_base_and_first_show_image_path() {
        let mut registry = crate::core::Registry::new();
        registry.register(SPEC);
        let mut operation = registry.start("https://fixtures.test/zoomify-base-href/product.html");

        let page = operation.missing_resources().unwrap().pop().unwrap();
        assert_eq!(
            page.request.uri,
            "https://fixtures.test/zoomify-base-href/product.html"
        );
        operation
            .provide(crate::core::ResourceResponse {
                id: page.id,
                bytes: br#"<!doctype html>
                    <html>
                      <head>
                        <base href="https://fixtures.test/zoomify-base-href/assets/">
                      </head>
                      <body>
                        <script>
                          Z.showImage("viewer", "maps/sample");
                          Z.showImage("viewer", "maps/missing");
                        </script>
                      </body>
                    </html>"#
                    .to_vec(),
            })
            .unwrap();

        let metadata = operation.missing_resources().unwrap().pop().unwrap();
        assert_eq!(
            metadata.request.uri,
            "https://fixtures.test/zoomify-base-href/assets/maps/sample/ImageProperties.xml"
        );
        operation
            .provide(crate::core::ResourceResponse {
                id: metadata.id,
                bytes: br#"<IMAGE_PROPERTIES WIDTH="512" HEIGHT="256" NUMTILES="2" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#.to_vec(),
            })
            .unwrap();

        let catalog = operation.finish().unwrap();
        let CatalogEntry::Ready(image) = &catalog.entries()[0] else {
            panic!("Zoomify metadata must produce a ready image")
        };
        let TileSource::Grid(plan) = &image.levels[0].source else {
            panic!("Zoomify levels must be grids")
        };
        assert_eq!(
            plan.tiles_row_major().next().unwrap().unwrap().request.uri,
            "https://fixtures.test/zoomify-base-href/assets/maps/sample/TileGroup0/0-0-0.jpg"
        );
    }

    #[test]
    fn viewer_declarations_match_dezoomify_fixtures_and_capture() {
        let cases = [
            (
                r#"<script>var zoomifyImagePath=/zoomify";</script>"#,
                "/zoomify",
            ),
            (
                r#"<script>showImage("viewer", "/zoomify");</script>"#,
                "/zoomify",
            ),
            (
                r#"<script>showImage(viewer, "/zoomify");</script>"#,
                "/zoomify",
            ),
            (
                r#"<script>Z.showImage("viewer", "https://example.com/proxy/IMAGE_ID/");</script>"#,
                "https://example.com/proxy/IMAGE_ID/",
            ),
        ];
        for (html, expected) in cases {
            assert_eq!(extract_image_path(html).as_deref(), Some(expected));
        }
    }

    #[test]
    fn first_viewer_declaration_wins_in_document_order() {
        let html = r#"
            <script>zoomifyImagePath=/first";</script>
            <script>Z.showImage("viewer", "/second");</script>
        "#;
        assert_eq!(extract_image_path(html).as_deref(), Some("/first"));
    }

    #[test]
    fn unrelated_pages_are_rejected() {
        let mut registry = crate::core::Registry::new();
        registry.register(SPEC);
        let mut operation = registry.start("https://example.com/page");
        let page = operation.missing_resources().unwrap().pop().unwrap();
        let error = operation
            .provide(crate::core::ResourceResponse {
                id: page.id,
                bytes: b"<html><body>ordinary page</body></html>".to_vec(),
            })
            .unwrap_err();
        assert!(matches!(error, DiscoveryError::NoCandidateAccepted { .. }));
    }

    fn ready_image(url: &str, contents: &[u8]) -> ImageDescriptor {
        match load_catalog(url, contents)
            .unwrap()
            .into_entries()
            .pop()
            .unwrap()
        {
            CatalogEntry::Ready(image) => image,
            CatalogEntry::Deferred(_) => panic!("Zoomify is resolved"),
        }
    }

    fn levels(url: &str, contents: &[u8]) -> Vec<LevelDescriptor> {
        ready_image(url, contents).levels
    }

    #[test]
    fn panorama_preserves_tile_order_and_normalized_levels() {
        let contents = br#"<IMAGE_PROPERTIES WIDTH="174550" HEIGHT="16991" NUMTILES="61284" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#;
        let levels = levels("http://x.fr/y/ImageProperties.xml?t", contents);
        assert_eq!(levels.len(), 11);
        assert!(
            levels
                .windows(2)
                .all(|pair| pair[0].source.image_size().unwrap().area()
                    <= pair[1].source.image_size().unwrap().area())
        );
        let TileSource::Grid(plan) = &levels[3].source else {
            unreachable!()
        };
        let urls: Vec<_> = plan
            .tiles_row_major()
            .take(6)
            .map(Result::unwrap)
            .map(|tile| tile.request.uri)
            .collect();
        assert_eq!(
            urls,
            vec![
                "http://x.fr/y/TileGroup0/3-0-0.jpg",
                "http://x.fr/y/TileGroup0/3-1-0.jpg",
                "http://x.fr/y/TileGroup0/3-2-0.jpg",
                "http://x.fr/y/TileGroup0/3-3-0.jpg",
                "http://x.fr/y/TileGroup0/3-4-0.jpg",
                "http://x.fr/y/TileGroup0/3-5-0.jpg"
            ]
        );
    }

    #[test]
    fn tile_group_boundary_matches_dezoomify() {
        let image = ready_image(
            "https://fixtures.test/zoomify/multiple-groups/ImageProperties.xml",
            br#"<IMAGE_PROPERTIES WIDTH="4096" HEIGHT="4096" NUMTILES="341" VERSION="1.8" TILESIZE="256" />"#,
        );
        let TileSource::Grid(plan) = &image.levels[4].source else {
            unreachable!()
        };
        let urls: Vec<_> = plan
            .tiles_row_major()
            .skip(170)
            .take(2)
            .map(Result::unwrap)
            .map(|tile| tile.request.uri)
            .collect();
        assert_eq!(
            urls,
            [
                "https://fixtures.test/zoomify/multiple-groups/TileGroup0/4-10-10.jpg",
                "https://fixtures.test/zoomify/multiple-groups/TileGroup1/4-11-10.jpg",
            ]
        );
    }

    #[test]
    fn full_resolution_numtiles_keeps_every_level_in_tile_group_zero() {
        let image = ready_image(
            "https://fixtures.test/zoomify-full-numtiles/ImageProperties.xml",
            br#"<IMAGE_PROPERTIES WIDTH="10240" HEIGHT="1792" NUMTILES="280" VERSION="1.8" TILESIZE="256" />"#,
        );
        assert!(image.warnings.is_empty());
        let TileSource::Grid(plan) = &image.levels.last().unwrap().source else {
            unreachable!()
        };
        let urls: Vec<_> = plan
            .tiles_row_major()
            .map(Result::unwrap)
            .map(|tile| tile.request.uri)
            .collect();
        assert_eq!(urls.len(), 280);
        assert!(urls.iter().all(|url| url.contains("/TileGroup0/")));
        assert!(urls.iter().any(|url| url.ends_with("/6-16-6.jpg")));
    }

    #[test]
    fn titles_tile_groups_and_warnings_are_retained() {
        let contents = br#"<IMAGE_PROPERTIES WIDTH="12000" HEIGHT="9788" NUMTILES="2477" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#;
        let image = ready_image(
            "http://example.com/images/manuscript123/ImageProperties.xml",
            contents,
        );
        assert_eq!(image.title.as_deref(), Some("manuscript123"));
        let TileSource::Grid(plan) = &image.levels[5].source else {
            unreachable!()
        };
        let urls: std::collections::HashSet<_> = plan
            .tiles_row_major()
            .map(Result::unwrap)
            .map(|tile| tile.request.uri)
            .collect();
        assert!(urls.contains("http://example.com/images/manuscript123/TileGroup1/5-0-14.jpg"));
        assert!(urls.contains("http://example.com/images/manuscript123/TileGroup2/5-0-15.jpg"));
        let image = ready_image(
            "https://library.example.edu/viewer/book_of_kells/ImageProperties.xml?cache=false",
            br#"<IMAGE_PROPERTIES WIDTH="2000" HEIGHT="3000" NUMTILES="100" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#,
        );
        assert_eq!(image.title.as_deref(), Some("book_of_kells"));
        let image = ready_image(
            "http://example.com/ImageProperties.xml",
            br#"<IMAGE_PROPERTIES WIDTH="500" HEIGHT="500" NUMTILES="9" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#,
        );
        assert_eq!(image.title.as_deref(), Some("example.com"));
        let contents = br#"<IMAGE_PROPERTIES WIDTH="500" HEIGHT="500" NUMTILES="9" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#;
        let image = ready_image("http://example.com/ImageProperties.xml", contents);
        assert_eq!(
            image.warnings,
            ["Zoomify tile count mismatch: computed 5, metadata declares 9"]
        );
    }

    #[test]
    fn full_resolution_only_numtiles_uses_the_full_level() {
        let image = ready_image(
            "https://fixtures.test/zoomify-full-numtiles/ImageProperties.xml",
            br#"<IMAGE_PROPERTIES WIDTH="10240" HEIGHT="1792" NUMTILES="280" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#,
        );
        let TileSource::Grid(plan) = &image.levels.last().unwrap().source else {
            unreachable!()
        };
        assert_eq!(plan.count(), 280);
        let first = plan.tiles_row_major().next().unwrap().unwrap();
        assert_eq!(
            first.request.uri,
            "https://fixtures.test/zoomify-full-numtiles/TileGroup0/6-0-0.jpg"
        );
        let urls: Vec<_> = plan
            .tiles_row_major()
            .map(Result::unwrap)
            .map(|tile| tile.request.uri)
            .collect();
        assert!(urls.iter().all(|url| url.contains("/TileGroup0/")));
        assert!(urls.iter().any(|url| url.ends_with("/6-16-6.jpg")));
    }
}
