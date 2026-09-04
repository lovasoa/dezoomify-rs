//! Pure discovery for Zoomify viewers and `ImageProperties.xml` pyramids.

use std::sync::{Arc, LazyLock};

use image_properties::ImageProperties;
use regex::{Regex, bytes::Regex as BytesRegex};
use url::Url;

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryContext, DiscoveryError, DiscoveryMatch, DiscoveryRoute,
    DiscoveryStep, Grid, ImageCatalog, ImageDescriptor, LevelDescriptor, Request, StableId,
    resolve_relative,
};

mod image_properties;

const ROUTES: &[DiscoveryRoute] = &[
    DiscoveryMatch::UrlPredicate(is_tile_url).map_url(tile_metadata),
    DiscoveryMatch::UrlSuffix("ImageProperties.xml").then(extract_catalog),
    DiscoveryMatch::UrlPredicate(is_special_zoomify_url).map_url(special_image_properties),
    DiscoveryMatch::UrlPredicate(is_fluid_broker_url).then(extract_fluid_image_properties_url),
    DiscoveryMatch::ContentPredicate(contains_zoomify_declaration)
        .then(extract_image_properties_url),
    DiscoveryMatch::ContentPredicate(contains_fluid_accessnumber).then(follow_fluid_broker),
    ngv::ROUTE,
    DiscoveryMatch::ContentPredicate(contains_iframe).then(follow_iframe),
];

pub const SPEC: DezoomerSpec = DezoomerSpec::new("zoomify", ROUTES).preferring(is_zoomify_url);

static SHOW_IMAGE_RE: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(r#"(?i)(?:\bZ\s*\.\s*)?\bshowImage\s*\([^,]*,\s*["'](?P<image>[^"']+)["']"#)
        .expect("constant Zoomify showImage pattern")
});

static FLASH_IMAGE_PATH_RE: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(
        r#"(?i)\bzoomifyImagePath\s*=\s*(?:["'](?P<image>[^"']*)["']|(?P<bare>[^'"&\s]+))"#,
    )
    .expect("constant Zoomify FlashVars pattern")
});

static TILE_SERVICE_RE: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(
        r#"(?is)\btype["']?\s*:\s*["']zoomifytileservice["'].*?\btilesUrl["']?\s*:\s*["'](?P<image>[^"']+)"#,
    )
    .expect("constant Zoomify tile service pattern")
});

static OPENLAYERS_SOURCE_RE: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(
        r#"(?is)<[^>]*\bclass\s*=\s*["'][^"']*\bete-openlayers-src\b[^"']*["'][^>]*>\s*(?P<image>.*?)\s*</[^>]+>"#,
    )
    .expect("constant OpenLayers source pattern")
});

static URL_ELEMENT_RE: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(r"(?is)<url\b[^>]*>\s*(?P<image>.*?)\s*</url\s*>")
        .expect("constant Zoomify URL element pattern")
});

static ACCESSNUMBER_RE: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(r#"(?i)\baccessnumber\s*=\s*(?P<id>[^"&\s']+)"#)
        .expect("constant Fluid Engage access number pattern")
});

static FLUID_IMAGE_PATH_RE: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(
        r#"(?is)<imagefile\b[^>]*\bformat\s*=\s*["']?zoomify["']?[^>]*>\s*(?P<image>.*?)\s*</imagefile\s*>"#,
    )
    .expect("constant Fluid Engage image path pattern")
});

static IFRAME_RE: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(r#"(?is)<i?frame\b[^>]*\bsrc\s*=\s*["'](?P<iframe>[^"']+)["']"#)
        .expect("constant iframe source pattern")
});

static HTML_BASE_RE: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(r#"(?is)<base\s+[^>]*\bhref\s*=\s*["'](?P<base>[^"']*)"#)
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
    uri.contains("/ImageProperties.xml")
        || ngv::prefers(uri)
        || is_tile_url(uri)
        || is_special_zoomify_url(uri)
}

fn is_special_zoomify_url(uri: &str) -> bool {
    uri.contains("biblio.unibe.ch/web-apps/maps/zoomify.php")
        || uri.contains("bspe-p-pub.paris.fr/MDBGED/zoomify-BFS.aspx")
        || uri.contains("artandarchitecture.org.uk/images/zoom/")
}

fn special_image_properties(input: &str) -> Result<Request, DiscoveryError> {
    let url = Url::parse(input)
        .map_err(|_| DiscoveryError::Session("invalid special Zoomify URL".into()))?;
    Ok(Request::new(format!(
        "{}/zoomify/ImageProperties.xml",
        url.origin().ascii_serialization()
    )))
}

fn is_fluid_broker_url(uri: &str) -> bool {
    uri.to_ascii_lowercase()
        .contains("/scripts/xmlbroker.new.php")
}

fn contains_fluid_accessnumber(contents: &[u8]) -> bool {
    ACCESSNUMBER_RE.is_match(contents)
}

fn follow_fluid_broker(
    _: &DiscoveryContext<'_>,
    resource: crate::core::DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let access_number = ACCESSNUMBER_RE
        .captures(resource.bytes())
        .and_then(|captures| capture_text(&captures, "id"))
        .ok_or_else(|| DiscoveryError::Session("Fluid page lacks an access number".into()))?;
    let broker =
        format!("/scripts/XMLBroker.new.php?Lang=2&contentType=IMAGES&contentID={access_number}");
    Ok(DiscoveryStep::Follow(Request::new(resolve_relative(
        resource.final_uri(),
        &broker,
    ))))
}

fn extract_fluid_image_properties_url(
    _: &DiscoveryContext<'_>,
    resource: crate::core::DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let image_path = FLUID_IMAGE_PATH_RE
        .captures(resource.bytes())
        .and_then(|captures| capture_text(&captures, "image"))
        .map(|path| path.trim().to_owned())
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            DiscoveryError::Session("Fluid XML does not declare a Zoomify image path".into())
        })?;
    let image_uri = resolve_relative(resource.final_uri(), &image_path);
    Ok(DiscoveryStep::Follow(Request::new(append_path_component(
        &image_uri,
        "ImageProperties.xml",
    ))))
}

fn contains_iframe(contents: &[u8]) -> bool {
    IFRAME_RE.is_match(contents)
}

fn follow_iframe(
    _: &DiscoveryContext<'_>,
    resource: crate::core::DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let iframe = IFRAME_RE
        .captures(resource.bytes())
        .and_then(|captures| capture_text(&captures, "iframe"))
        .ok_or_else(|| DiscoveryError::Session("Zoomify page lacks an iframe URL".into()))?;
    Ok(DiscoveryStep::Follow(Request::new(resolve_relative(
        resource.final_uri(),
        &iframe,
    ))))
}

fn extract_image_properties_url(
    _: &DiscoveryContext<'_>,
    resource: crate::core::DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let image_path = extract_image_path(resource.bytes()).ok_or_else(|| {
        DiscoveryError::Session("Zoomify viewer page does not declare an image path".into())
    })?;
    let page_base_uri = extract_html_base(resource.bytes()).map_or_else(
        || resource.final_uri().to_owned(),
        |base| resolve_relative(resource.final_uri(), &base),
    );
    let image_uri = resolve_relative(&page_base_uri, &image_path);
    Ok(DiscoveryStep::Follow(Request::new(append_path_component(
        &image_uri,
        "ImageProperties.xml",
    ))))
}

fn extract_catalog(
    _: &DiscoveryContext<'_>,
    resource: crate::core::DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    load_catalog(resource.uri(), resource.bytes()).map(DiscoveryStep::Complete)
}

mod ngv;

fn contains_zoomify_declaration(contents: &[u8]) -> bool {
    SHOW_IMAGE_RE.is_match(contents)
        || FLASH_IMAGE_PATH_RE.is_match(contents)
        || TILE_SERVICE_RE.is_match(contents)
        || OPENLAYERS_SOURCE_RE.is_match(contents)
        || URL_ELEMENT_RE.is_match(contents)
}

fn append_path_component(uri: &str, component: &str) -> String {
    let suffix_start = uri.find(['?', '#']).unwrap_or(uri.len());
    let (path, suffix) = uri.split_at(suffix_start);
    format!("{}/{component}{suffix}", path.trim_end_matches('/'))
}

fn extract_image_path(html: &[u8]) -> Option<String> {
    let show_image = SHOW_IMAGE_RE
        .captures_iter(html)
        .find_map(|captures| Some((captures.get(0)?.start(), capture_text(&captures, "image")?)));
    let flash = FLASH_IMAGE_PATH_RE
        .captures_iter(html)
        .find_map(|captures| {
            Some((
                captures.get(0)?.start(),
                capture_text(&captures, "image").or_else(|| capture_text(&captures, "bare"))?,
            ))
        });
    let tile_service = TILE_SERVICE_RE
        .captures_iter(html)
        .filter_map(|captures| Some((captures.get(0)?.start(), capture_text(&captures, "image")?)))
        .min_by_key(|(offset, _)| *offset);
    if let Some((_, path)) = [show_image, flash, tile_service]
        .into_iter()
        .flatten()
        .min_by_key(|(offset, _)| *offset)
    {
        return Some(path);
    }
    OPENLAYERS_SOURCE_RE
        .captures(html)
        .and_then(|captures| capture_text(&captures, "image"))
        .or_else(|| {
            URL_ELEMENT_RE
                .captures(html)
                .and_then(|captures| capture_text(&captures, "image"))
        })
        .map(|path| path.trim().to_owned())
        .filter(|path| !path.is_empty())
}

fn capture_text(captures: &regex::bytes::Captures<'_>, name: &str) -> Option<String> {
    captures
        .name(name)
        .map(|capture| String::from_utf8_lossy(capture.as_bytes()).replace("&amp;", "&"))
}

fn extract_html_base(html: &[u8]) -> Option<String> {
    HTML_BASE_RE
        .captures(html)
        .and_then(|captures| capture_text(&captures, "base"))
}

#[allow(clippy::unnecessary_wraps)]
fn tile_metadata(input: &str) -> Result<Request, DiscoveryError> {
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
    Ok(Request::new(uri))
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
    use crate::core::discovery::DiscoveryOperation;
    use crate::core::{ResourceResponse, TileSource};

    const XML: &[u8] = br#"<IMAGE_PROPERTIES WIDTH="512" HEIGHT="256" NUMTILES="2" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#;

    fn operation(uri: &str) -> DiscoveryOperation {
        let mut registry = crate::core::Registry::new();
        registry.register(SPEC);
        registry.start(uri)
    }

    fn provide_next(operation: &mut DiscoveryOperation, bytes: &[u8]) -> String {
        let need = operation.missing_resources().unwrap().pop().unwrap();
        let uri = need.request.uri.clone();
        operation
            .provide(ResourceResponse::new(need.id, bytes))
            .unwrap();
        uri
    }

    fn provide_next_at(
        operation: &mut DiscoveryOperation,
        bytes: &[u8],
        final_uri: &str,
    ) -> String {
        let need = operation.missing_resources().unwrap().pop().unwrap();
        let uri = need.request.uri.clone();
        operation
            .provide(ResourceResponse::new(need.id, bytes).with_final_uri(final_uri))
            .unwrap();
        uri
    }

    fn first_tile(operation: DiscoveryOperation) -> String {
        let catalog = operation.finish().unwrap();
        let CatalogEntry::Ready(image) = &catalog.entries()[0] else {
            panic!("Zoomify metadata must produce a ready image")
        };
        let TileSource::Grid(plan) = &image.levels[0].source else {
            panic!("Zoomify levels must be grids")
        };
        plan.tiles_row_major().next().unwrap().unwrap().request.uri
    }

    fn discover_viewer(page_uri: &str, page: &[u8]) -> (String, String) {
        let mut operation = operation(page_uri);
        assert_eq!(provide_next(&mut operation, page), page_uri);
        let metadata = provide_next(&mut operation, XML);
        (metadata, first_tile(operation))
    }

    #[test]
    fn tile_urls_request_sibling_metadata() {
        let mut operation =
            operation("https://example.com/images/book/TileGroup0/3-0-0.jpg?token=secret");
        assert_eq!(
            provide_next(&mut operation, XML),
            "https://example.com/images/book/ImageProperties.xml?token=secret"
        );
        assert_eq!(
            first_tile(operation),
            "https://example.com/images/book/TileGroup0/0-0-0.jpg"
        );
    }

    #[test]
    fn viewer_pages_respect_html_base_and_first_show_image_path() {
        let (metadata, tile) = discover_viewer(
            "https://fixtures.test/zoomify-base-href/product.html",
            br#"<base href="https://fixtures.test/zoomify-base-href/assets/">
                <script>
                    Z.showImage("viewer", "maps/sample");
                    Z.showImage("viewer", "maps/missing");
                </script>"#,
        );
        assert_eq!(
            metadata,
            "https://fixtures.test/zoomify-base-href/assets/maps/sample/ImageProperties.xml"
        );
        assert_eq!(
            tile,
            "https://fixtures.test/zoomify-base-href/assets/maps/sample/TileGroup0/0-0-0.jpg"
        );
    }

    #[test]
    fn viewer_pages_resolve_relative_paths_against_the_redirect_target() {
        let mut operation = operation("https://museum.example/object/12");
        assert_eq!(
            provide_next_at(
                &mut operation,
                br#"<script>Z.showImage("viewer", "tiles");</script>"#,
                "https://cdn.example/viewer/12/index.html",
            ),
            "https://museum.example/object/12"
        );
        assert_eq!(
            provide_next(&mut operation, XML),
            "https://cdn.example/viewer/12/tiles/ImageProperties.xml"
        );
    }

    #[test]
    fn legacy_path_elements_are_supported() {
        for page in [
            br#"<span class="ete-openlayers-src">/zoomify</span>"#.as_slice(),
            br"<url>/zoomify</url>",
        ] {
            assert_eq!(
                discover_viewer("https://fixtures.test/viewer.html", page),
                (
                    "https://fixtures.test/zoomify/ImageProperties.xml".into(),
                    "https://fixtures.test/zoomify/TileGroup0/0-0-0.jpg".into(),
                )
            );
        }
    }

    #[test]
    fn fluid_access_number_uses_the_xml_broker() {
        let input = "https://fixtures.test/zoomify/fluid.html";
        let mut operation = operation(input);
        assert_eq!(
            provide_next(&mut operation, b"var accessnumber=fixture-access-number\";"),
            input
        );
        let broker = operation.missing_resources().unwrap().pop().unwrap();
        assert_eq!(
            broker.request.uri,
            "https://fixtures.test/scripts/XMLBroker.new.php?Lang=2&contentType=IMAGES&contentID=fixture-access-number"
        );
        operation
            .provide(ResourceResponse::new(
                broker.id,
                br#"<response><imagefile format="zoomify">/zoomify</imagefile></response>"#,
            ))
            .unwrap();
        assert_eq!(
            provide_next(&mut operation, XML),
            "https://fixtures.test/zoomify/ImageProperties.xml"
        );
        assert_eq!(
            first_tile(operation),
            "https://fixtures.test/zoomify/TileGroup0/0-0-0.jpg"
        );
    }

    #[test]
    fn iframe_pages_follow_the_embedded_viewer() {
        let input = "https://fixtures.test/zoomify/iframe-parent.html";
        let mut operation = operation(input);
        assert_eq!(
            provide_next(
                &mut operation,
                br#"<iframe src="/zoomify/iframe-child.html"></iframe>"#
            ),
            input
        );
        assert_eq!(
            provide_next(
                &mut operation,
                br#"<script>showImage("viewer", "/zoomify");</script>"#,
            ),
            "https://fixtures.test/zoomify/iframe-child.html"
        );
        assert_eq!(
            provide_next(&mut operation, XML),
            "https://fixtures.test/zoomify/ImageProperties.xml"
        );
        assert_eq!(
            first_tile(operation),
            "https://fixtures.test/zoomify/TileGroup0/0-0-0.jpg"
        );
    }

    #[test]
    fn known_zoomify_sites_map_to_their_image_properties() {
        for (input, expected) in [
            (
                "https://biblio.unibe.ch/web-apps/maps/zoomify.php?col=ryh&pic=Ryh_7906_6",
                "https://biblio.unibe.ch/zoomify/ImageProperties.xml",
            ),
            (
                "https://bspe-p-pub.paris.fr/MDBGED/zoomify-BFS.aspx?edid=23143&edfindex=0",
                "https://bspe-p-pub.paris.fr/zoomify/ImageProperties.xml",
            ),
            (
                "https://www.artandarchitecture.org.uk/images/zoom/c462969579cd09dd4ccb690d0e43018757fa2df2.html",
                "https://www.artandarchitecture.org.uk/zoomify/ImageProperties.xml",
            ),
        ] {
            let mut operation = operation(input);
            assert_eq!(
                operation
                    .missing_resources()
                    .unwrap()
                    .pop()
                    .unwrap()
                    .request
                    .uri,
                expected
            );
        }
    }

    #[test]
    fn redirected_metadata_keeps_the_requested_tile_base() {
        let mut operation = operation("https://origin.example/book/ImageProperties.xml");
        let metadata = provide_next_at(
            &mut operation,
            XML,
            "https://cdn.example/metadata/content.xml",
        );
        assert_eq!(metadata, "https://origin.example/book/ImageProperties.xml");
        assert_eq!(
            first_tile(operation),
            "https://origin.example/book/TileGroup0/0-0-0.jpg"
        );
    }

    #[test]
    fn signed_proxy_remains_the_tile_base() {
        let (metadata, tile) = discover_viewer(
            "https://museum.example/viewer/object",
            br#"<script>Z.showImage("viewer", "https://museum.example/proxy/OBJECT_ID/");</script>"#,
        );
        assert_eq!(
            metadata,
            "https://museum.example/proxy/OBJECT_ID/ImageProperties.xml"
        );
        assert_eq!(
            tile,
            "https://museum.example/proxy/OBJECT_ID/TileGroup0/0-0-0.jpg"
        );
    }

    #[test]
    fn extracts_general_zoomify_declarations() {
        for (page, expected) in [
            (r#"zoomifyImagePath=/zoomify";"#, "/zoomify"),
            (r#"zoomifyImagePath = "/zoomify";"#, "/zoomify"),
            (r#"showImage("viewer", "/zoomify");"#, "/zoomify"),
            (r#"showImage(viewer, "/zoomify");"#, "/zoomify"),
            (
                r#"Z.showImage("viewer", "https://example.com/proxy/IMAGE_ID/");"#,
                "https://example.com/proxy/IMAGE_ID/",
            ),
            (
                r#"{"type": "zoomifytileservice", "tilesUrl": "/zoomify"}"#,
                "/zoomify",
            ),
        ] {
            assert_eq!(
                extract_image_path(page.as_bytes()).as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn unrelated_pages_are_rejected() {
        let mut operation = operation("https://example.com/page");
        let need = operation.missing_resources().unwrap().pop().unwrap();
        let error = operation
            .provide(ResourceResponse::new(
                need.id,
                b"<html><body>ordinary page</body></html>",
            ))
            .unwrap_err();
        assert!(matches!(error, DiscoveryError::NoCandidateAccepted { .. }));
    }

    #[test]
    fn generic_var_url_pages_are_not_treated_as_ngv() {
        let mut operation = operation("https://example.com/page");
        let need = operation.missing_resources().unwrap().pop().unwrap();
        let error = operation
            .provide(ResourceResponse::new(
                need.id,
                br"<script>var url = '/zoomify';</script>",
            ))
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
            [
                "http://x.fr/y/TileGroup0/3-0-0.jpg",
                "http://x.fr/y/TileGroup0/3-1-0.jpg",
                "http://x.fr/y/TileGroup0/3-2-0.jpg",
                "http://x.fr/y/TileGroup0/3-3-0.jpg",
                "http://x.fr/y/TileGroup0/3-4-0.jpg",
                "http://x.fr/y/TileGroup0/3-5-0.jpg",
            ]
        );
    }

    #[test]
    fn titles_tile_groups_and_warnings_are_retained() {
        let image = ready_image(
            "http://example.com/images/manuscript123/ImageProperties.xml",
            br#"<IMAGE_PROPERTIES WIDTH="12000" HEIGHT="9788" NUMTILES="2477" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#,
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
            "http://example.com/ImageProperties.xml",
            br#"<IMAGE_PROPERTIES WIDTH="500" HEIGHT="500" NUMTILES="9" NUMIMAGES="1" VERSION="1.8" TILESIZE="256"/>"#,
        );
        assert_eq!(image.title.as_deref(), Some("example.com"));
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
        let urls: Vec<_> = plan
            .tiles_row_major()
            .map(Result::unwrap)
            .map(|tile| tile.request.uri)
            .collect();
        assert!(urls.iter().all(|url| url.contains("/TileGroup0/")));
        assert!(urls.iter().any(|url| url.ends_with("/6-16-6.jpg")));
    }
}
