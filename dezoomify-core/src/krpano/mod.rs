//! Pure, resumable discovery for krpano panoramas.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, LazyLock};

use itertools::Itertools;
use regex::Regex;

use krpano_decrypt::{decrypt_xml, is_encrypted_xml};
use krpano_metadata::{KrpanoMetadata, TemplateString, TemplateStringPart, XY};

use crate::Vec2d;
use crate::core::discovery::DiscoveryEvent;
use crate::core::resolve_relative;
use crate::core::tile_plan::RectangularSource;
use crate::core::{
    CatalogEntry, DiscoveryDiagnostic, DiscoveryError, DiscoveryInput, DiscoveryProgram,
    DiscoverySession, DiscoveryStep, ImageCatalog, ImageDescriptor, KnownTilePlan, LevelDescriptor,
    LevelPlan, ProcessingRecipe, Request, ResourceOutcome, ResourceRequest, StableId,
};
use crate::krpano::krpano_metadata::{ImageInfo, LevelDesc};

mod krpano_metadata;

/// The krpano metadata discovery program.
///
/// The program owns only parser state. The application supplies the requested
/// HTML, XML, and viewer-script bytes through [`DiscoverySession::advance`].
#[derive(Default)]
pub struct KrpanoDezoomer;

impl DiscoveryProgram for KrpanoDezoomer {
    fn start(&self, input: &DiscoveryInput) -> Box<dyn DiscoverySession> {
        Box::new(KrpanoSession {
            input_uri: input.uri.clone(),
            state: SessionState::Initial,
        })
    }
}

/// One operation-local HTML → XML → viewer-JS resolution chain.
struct KrpanoSession {
    input_uri: String,
    state: SessionState,
}

enum SessionState {
    Initial,
    WaitingInitial,
    NeedXml {
        xml_uri: String,
        viewer_js: Vec<u8>,
        remaining_js_uris: Vec<String>,
    },
    NeedViewerJs {
        xml_uri: String,
        xml_contents: Vec<u8>,
        remaining_js_uris: Vec<String>,
    },
    Complete,
}

impl DiscoverySession for KrpanoSession {
    fn advance(&mut self, event: DiscoveryEvent<'_>) -> Result<DiscoveryStep, DiscoveryError> {
        match event {
            DiscoveryEvent::Start if matches!(self.state, SessionState::Initial) => {
                self.state = SessionState::WaitingInitial;
                Ok(need(&self.input_uri))
            }
            DiscoveryEvent::Start => Err(DiscoveryError::Session(
                "krpano session started twice".into(),
            )),
            DiscoveryEvent::Resource(ResourceOutcome::Failure(failure)) => {
                self.handle_failure(failure.message.as_str())
            }
            DiscoveryEvent::Resource(ResourceOutcome::Response(response)) => {
                self.handle_response(&response.bytes)
            }
        }
    }
}

impl KrpanoSession {
    fn handle_failure(&mut self, message: &str) -> Result<DiscoveryStep, DiscoveryError> {
        let state = std::mem::replace(&mut self.state, SessionState::Complete);
        match state {
            SessionState::NeedViewerJs {
                xml_uri,
                xml_contents,
                mut remaining_js_uris,
            } => {
                if let Some(next_js_uri) = next_js_candidate(&mut remaining_js_uris) {
                    self.state = SessionState::NeedViewerJs {
                        xml_uri,
                        xml_contents,
                        remaining_js_uris,
                    };
                    Ok(need(&next_js_uri))
                } else {
                    Err(DiscoveryError::Session(format!(
                        "failed to download krpano viewer script: {message}"
                    )))
                }
            }
            SessionState::Initial | SessionState::WaitingInitial | SessionState::NeedXml { .. } => {
                Err(DiscoveryError::Session(format!(
                    "failed to download krpano metadata: {message}"
                )))
            }
            SessionState::Complete => Err(DiscoveryError::Session(
                "krpano session has already completed".into(),
            )),
        }
    }

    fn handle_response(&mut self, contents: &[u8]) -> Result<DiscoveryStep, DiscoveryError> {
        let state = std::mem::replace(&mut self.state, SessionState::Complete);
        match state {
            SessionState::WaitingInitial => self.handle_initial(contents),
            SessionState::Initial => Err(DiscoveryError::Session(
                "krpano session received a resource before it started".into(),
            )),
            SessionState::NeedXml {
                xml_uri,
                viewer_js,
                remaining_js_uris,
            } => self.handle_xml(contents, xml_uri, &viewer_js, remaining_js_uris),
            SessionState::NeedViewerJs {
                xml_uri,
                xml_contents,
                remaining_js_uris,
            } => self.handle_viewer_js(contents, xml_uri, xml_contents, remaining_js_uris),
            SessionState::Complete => Err(DiscoveryError::Session(
                "krpano session has already completed".into(),
            )),
        }
    }

    fn handle_initial(&mut self, contents: &[u8]) -> Result<DiscoveryStep, DiscoveryError> {
        if !looks_like_krpano_xml(contents) && looks_like_viewer_js(contents) {
            let xml_uri = sibling_uri(&self.input_uri, "tour.xml");
            self.state = SessionState::NeedXml {
                xml_uri: xml_uri.clone(),
                viewer_js: contents.to_vec(),
                remaining_js_uris: Vec::new(),
            };
            return Ok(need(&xml_uri));
        }

        if !looks_like_krpano_xml(contents) && looks_like_krpano_html(contents) {
            let html = String::from_utf8_lossy(contents);
            let remaining_js_uris = extract_js_candidates_from_html(&html, &self.input_uri);
            let xml_uri = extract_xml_from_embedpano(&html).map_or_else(
                || sibling_uri(&self.input_uri, "tour.xml"),
                |reference| resolve_relative(&self.input_uri, &reference),
            );
            self.state = SessionState::NeedXml {
                xml_uri: xml_uri.clone(),
                viewer_js: Vec::new(),
                remaining_js_uris,
            };
            return Ok(need(&xml_uri));
        }

        if is_encrypted_xml(contents) {
            if let Ok(decrypted) = decrypt_xml(contents, None) {
                return self.complete(&self.input_uri.clone(), &decrypted);
            }
            let mut candidates = viewer_js_candidates_for_xml(&self.input_uri);
            let viewer_uri = next_js_candidate(&mut candidates)
                .unwrap_or_else(|| sibling_uri(&self.input_uri, "tour.js"));
            self.state = SessionState::NeedViewerJs {
                xml_uri: self.input_uri.clone(),
                xml_contents: contents.to_vec(),
                remaining_js_uris: candidates,
            };
            return Ok(need(&viewer_uri));
        }

        if looks_like_krpano_xml(contents) {
            return self.complete(&self.input_uri.clone(), contents);
        }

        Ok(DiscoveryStep::Reject(DiscoveryDiagnostic::from(
            "not krpano HTML, viewer JavaScript, or XML metadata",
        )))
    }

    fn handle_xml(
        &mut self,
        contents: &[u8],
        xml_uri: String,
        viewer_js: &[u8],
        mut remaining_js_uris: Vec<String>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        if !is_encrypted_xml(contents) {
            return self.complete(&xml_uri, contents);
        }

        match decrypt_xml(contents, (!viewer_js.is_empty()).then_some(viewer_js)) {
            Ok(decrypted) => self.complete(&xml_uri, &decrypted),
            Err(error) => {
                let Some(viewer_uri) = next_js_candidate(&mut remaining_js_uris) else {
                    return Err(DiscoveryError::Session(format!(
                        "unable to decrypt krpano XML: {error}"
                    )));
                };
                self.state = SessionState::NeedViewerJs {
                    xml_uri,
                    xml_contents: contents.to_vec(),
                    remaining_js_uris,
                };
                Ok(need(&viewer_uri))
            }
        }
    }

    fn handle_viewer_js(
        &mut self,
        contents: &[u8],
        xml_uri: String,
        xml_contents: Vec<u8>,
        mut remaining_js_uris: Vec<String>,
    ) -> Result<DiscoveryStep, DiscoveryError> {
        let viewer_js = extract_viewer_js(contents).unwrap_or_else(|| contents.to_vec());
        match decrypt_xml(&xml_contents, Some(&viewer_js)) {
            Ok(decrypted) => self.complete(&xml_uri, &decrypted),
            Err(error) => {
                let Some(viewer_uri) = next_js_candidate(&mut remaining_js_uris) else {
                    return Err(DiscoveryError::Session(format!(
                        "unable to decrypt krpano XML: {error}"
                    )));
                };
                self.state = SessionState::NeedViewerJs {
                    xml_uri,
                    xml_contents,
                    remaining_js_uris,
                };
                Ok(need(&viewer_uri))
            }
        }
    }

    fn complete(&mut self, uri: &str, bytes: &[u8]) -> Result<DiscoveryStep, DiscoveryError> {
        let catalog = load_catalog(uri, bytes)?;
        self.state = SessionState::Complete;
        Ok(DiscoveryStep::Complete(catalog))
    }
}

fn need(uri: &str) -> DiscoveryStep {
    DiscoveryStep::Need(ResourceRequest::new(uri))
}

/// True if the content looks like a krpano XML file rather than HTML.
fn looks_like_krpano_xml(contents: &[u8]) -> bool {
    let contents = contents.strip_prefix(b"\xef\xbb\xbf").unwrap_or(contents);
    let text = String::from_utf8_lossy(contents);
    let trimmed = text.trim_start();
    trimmed.starts_with("<?xml") || trimmed.to_ascii_lowercase().starts_with("<krpano")
}

/// True if the content has krpano-specific HTML evidence.
fn looks_like_krpano_html(contents: &[u8]) -> bool {
    let text = String::from_utf8_lossy(contents);
    let lower = text.to_ascii_lowercase();
    lower.contains("embedpano(")
        || lower.contains("createpanoviewer(")
        || (lower.contains("<script") && (lower.contains("krpano") || lower.contains("tour.js")))
}

/// True if the content looks like a krpano viewer JavaScript file.
fn looks_like_viewer_js(contents: &[u8]) -> bool {
    let contents = contents.strip_prefix(b"\xef\xbb\xbf").unwrap_or(contents);
    if contents.starts_with(b"/*") {
        return contents[..contents.len().min(512)]
            .windows(6)
            .any(|window| window == b"krpano");
    }
    if contents.starts_with(b"function ") {
        let window = &contents[..contents.len().min(8192)];
        return window.windows(6).any(|part| part == b"krpano")
            || window.windows(9).any(|part| part == b"embedpano")
            || window.windows(16).any(|part| part == b"createPanoViewer");
    }
    false
}

/// Extract and rank viewer JavaScript candidates from a krpano HTML page.
fn extract_js_candidates_from_html(html: &str, html_uri: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for (index, tag) in SCRIPT_TAG_RE.find_iter(html).enumerate() {
        let Some(src) = extract_src_attr(tag.as_str()) else {
            continue;
        };
        if !is_javascript_src(&src) {
            continue;
        }
        let uri = resolve_relative(html_uri, &src);
        if seen.insert(uri.clone()) {
            candidates.push(ScriptCandidate {
                uri,
                score: viewer_script_score(&src),
                index,
            });
        }
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.index.cmp(&right.index))
    });
    candidates
        .into_iter()
        .map(|candidate| candidate.uri)
        .collect()
}

fn extract_xml_from_embedpano(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower
        .find("embedpano(")
        .or_else(|| lower.find("createpanoviewer("))?;
    let body = &html[start..];
    let end = EMBEDPANO_END_RE.find(body)?;
    let params = &body[..end.end()];
    EMBEDPANO_XML_RE
        .captures(params)
        .map(|captures| captures[1].to_owned())
}

fn extract_src_attr(tag: &str) -> Option<String> {
    let captures = SCRIPT_SRC_RE.captures(tag)?;
    captures
        .get(1)
        .or_else(|| captures.get(2))
        .or_else(|| captures.get(3))
        .map(|capture| capture.as_str().to_owned())
}

fn extract_viewer_js(contents: &[u8]) -> Option<Vec<u8>> {
    if looks_like_viewer_js(contents) {
        return Some(contents.to_vec());
    }
    let text = String::from_utf8_lossy(contents);
    let start = text.find("<script>")?;
    let body = &text[start + 8..];
    let end = body.find("</script>")?;
    let script = body[..end].trim();
    looks_like_viewer_js(script.as_bytes()).then(|| script.as_bytes().to_vec())
}

fn viewer_js_candidates_for_xml(xml_uri: &str) -> Vec<String> {
    let path = xml_uri
        .split_once(['?', '#'])
        .map_or(xml_uri, |(path, _)| path);
    let stem = path
        .rsplit(['/', '\\'])
        .next()
        .and_then(|name| name.rsplit_once('.').map(|(stem, _)| stem))
        .filter(|stem| !stem.is_empty())
        .unwrap_or("tour");
    let mut candidates = vec![sibling_uri(xml_uri, &format!("{stem}.js"))];
    for fallback in ["tour.js", "krpano.js"] {
        let candidate = sibling_uri(xml_uri, fallback);
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn sibling_uri(uri: &str, filename: &str) -> String {
    let uri = uri.split_once(['?', '#']).map_or(uri, |(path, _)| path);
    let search_start = uri.find("://").map_or(0, |index| index + 3);
    match uri[search_start..].rfind(['/', '\\']) {
        Some(relative_index) => {
            let index = search_start + relative_index;
            format!("{}{}{filename}", &uri[..index], &uri[index..=index])
        }
        None if search_start > 0 => format!("{uri}/{filename}"),
        None => filename.to_owned(),
    }
}

#[derive(Debug)]
struct ScriptCandidate {
    uri: String,
    score: i32,
    index: usize,
}

static SCRIPT_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<script\b[^>]*>").expect("constant script tag regex"));
static SCRIPT_SRC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)(?:^|[\s<])src\s*=\s*(?:\"([^\"]*)\"|'([^']*)'|([^\s>]+))"#)
        .expect("constant script source regex")
});
static EMBEDPANO_END_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\}\s*\)").expect("constant embed closing regex"));
static EMBEDPANO_XML_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bxml[\"']?\s*:\s*[\"']([^\"']+)[\"']"#).expect("constant embed XML regex")
});

fn is_javascript_src(src: &str) -> bool {
    let path = src.split_once(['?', '#']).map_or(src, |(path, _)| path);
    path.rsplit(['/', '\\'])
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("js"))
}

fn viewer_script_score(src: &str) -> i32 {
    let lower = src.to_ascii_lowercase();
    let path = lower
        .split_once(['?', '#'])
        .map_or(lower.as_str(), |(path, _)| path);
    let filename = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let mut score = match filename {
        "tour.js" => 1_000,
        "krpano.js" => 950,
        _ => 0,
    };
    if filename != "tour.js" && filename != "krpano.js" {
        if filename.contains("krpano") {
            score += 850;
        }
        if filename.contains("pano") {
            score += 450;
        }
        if filename.contains("tour") {
            score += 400;
        }
        if filename.contains("viewer") {
            score += 250;
        }
    }
    if is_common_non_viewer_script(filename) || is_common_non_viewer_script(path) {
        score -= 1_000;
    }
    score
}

fn is_common_non_viewer_script(value: &str) -> bool {
    [
        "jquery",
        "analytics",
        "gtag",
        "googletagmanager",
        "matomo",
        "piwik",
        "bootstrap",
        "modernizr",
        "polyfill",
        "underscore",
        "lodash",
        "react",
        "vue",
        "angular",
        "runtime",
        "vendor",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn next_js_candidate(candidates: &mut Vec<String>) -> Option<String> {
    (!candidates.is_empty()).then(|| candidates.remove(0))
}

fn load_catalog(url: &str, contents: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let metadata = KrpanoMetadata::from_bytes(contents)
        .map_err(|error| DiscoveryError::Session(format!("unable to parse krpano XML: {error}")))?;
    let global_title = metadata.get_title().unwrap_or_default().to_owned();
    let mut entries = Vec::new();

    for (image_index, ImageInfo { image, name }) in metadata.into_image_iter().enumerate() {
        let root_tile_size = image.tilesize.map(Vec2d::square);
        let base_index = image.baseindex;
        let image_title = joined_nonempty([global_title.as_str(), name.as_ref()]);
        let mut levels = Vec::new();
        let mut warnings = Vec::new();

        for (source_index, source_level) in image.into_levels().enumerate() {
            for description in source_level.level_descriptions(None, source_index) {
                let LevelDesc {
                    name: shape_name,
                    size,
                    tilesize,
                    url: template,
                    level_index,
                } = match description {
                    Ok(description) => description,
                    Err(error) => {
                        warnings.push(format!("bad krpano level: {error}"));
                        continue;
                    }
                };
                let Some(tile_size) = tilesize.or(root_tile_size) else {
                    warnings.push("bad krpano level: missing tile size".into());
                    continue;
                };
                let level_number = level_index + base_index as usize;
                for (side_name, template) in template.all_sides(level_number) {
                    let ordinal = levels.len();
                    let id = StableId::new(format!(
                        "krpano:{image_index}:{source_index}:{ordinal}:{side_name}"
                    ));
                    let face = face_label(shape_name, side_name);
                    let source = KrpanoLevel {
                        base_url: Arc::from(url),
                        size,
                        tile_size,
                        base_index,
                        template,
                        label: format_level_label(shape_name, side_name, &name),
                        id: id.clone(),
                    };
                    levels.push(LevelDescriptor {
                        id,
                        title: level_title(&global_title, &name, &face),
                        size: Some(size),
                        tile_size: Some(tile_size),
                        scale_factor: None,
                        has_overlapping_tiles: false,
                        plan: LevelPlan::Known(KnownTilePlan::rectangular(source)),
                        warnings: Vec::new(),
                    });
                }
            }
        }

        entries.push(CatalogEntry::Ready(ImageDescriptor {
            id: StableId::new(format!("krpano:{image_index}")),
            title: image_title,
            format: StableId::new("krpano"),
            levels,
            warnings,
        }));
    }
    Ok(ImageCatalog::new(entries))
}


fn joined_nonempty<'a>(parts: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let title = parts.into_iter().filter(|part| !part.is_empty()).join(" ");
    (!title.is_empty()).then_some(title)
}

fn face_label(shape: &str, side: &str) -> String {
    [shape, side]
        .into_iter()
        .filter(|part| !part.is_empty())
        .join(" ")
}

fn level_title(global: &str, scene: &str, face: &str) -> Option<String> {
    let title = [global, scene, face]
        .into_iter()
        .filter(|part| !part.is_empty())
        .join(" ");
    (!title.is_empty()).then_some(title)
}

fn format_level_label(shape: &str, side: &str, scene: &str) -> String {
    ["Krpano", shape, side, scene]
        .into_iter()
        .filter(|part| !part.is_empty())
        .join(" ")
}

struct KrpanoLevel {
    base_url: Arc<str>,
    size: Vec2d,
    tile_size: Vec2d,
    base_index: u32,
    template: TemplateString<XY>,
    label: String,
    id: StableId,
}

impl fmt::Debug for KrpanoLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

impl RectangularSource for KrpanoLevel {
    fn level_id(&self) -> StableId {
        self.id.clone()
    }

    fn image_size(&self) -> Vec2d {
        self.size
    }

    fn tile_size(&self) -> Vec2d {
        self.tile_size
    }

    fn request(&self, cell: Vec2d) -> Request {
        use std::fmt::Write;
        let mut relative = String::new();
        for part in &self.template.0 {
            match part {
                TemplateStringPart::Literal(value) => relative += value,
                TemplateStringPart::Variable { padding, variable } => {
                    write!(
                        relative,
                        "{value:0padding$}",
                        value = self.base_index
                            + match variable {
                                XY::X => cell.x,
                                XY::Y => cell.y,
                            },
                        padding = *padding,
                    )
                    .expect("writing to String cannot fail");
                }
            }
        }
        Request::new(resolve_relative(&self.base_url, &relative))
    }

    fn processing(&self) -> ProcessingRecipe {
        ProcessingRecipe::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(catalog: ImageCatalog) -> ImageDescriptor {
        match catalog.into_entries().into_iter().next().unwrap() {
            CatalogEntry::Ready(image) => image,
            CatalogEntry::Deferred(_) => panic!("krpano XML is ready"),
        }
    }

    fn tile_requests(level: &LevelDescriptor, count: usize) -> Vec<(String, Vec2d)> {
        let plan = match &level.plan {
            LevelPlan::Known(plan) => plan,
            LevelPlan::Adaptive(_) => panic!("krpano plans are known"),
        };
        plan.cursor()
            .take_ready(count)
            .unwrap()
            .unwrap_or_default()
            .into_iter()
            .map(|tile| (tile.request.uri, tile.destination))
            .collect()
    }

    fn catalog_from_xml(uri: &str, contents: &[u8]) -> ImageCatalog {
        load_catalog(uri, contents).unwrap()
    }

    fn discover_single_resource(uri: &str, bytes: Vec<u8>) -> ImageCatalog {
        let input = DiscoveryInput::from(uri);
        let mut session = KrpanoDezoomer.start(&input);
        assert!(matches!(
            session.advance(DiscoveryEvent::Start).unwrap(),
            DiscoveryStep::Need(ResourceRequest { request })
                if request.uri == uri
        ));
        match session
            .advance(DiscoveryEvent::Resource(&ResourceOutcome::Response(
                crate::core::ResourceResponse {
                    id: crate::core::RequestId(0),
                    bytes
                },
            )))
            .unwrap()
        {
            DiscoveryStep::Complete(catalog) => catalog,
            _ => panic!("plain XML completes discovery"),
        }
    }

    #[test]
    fn test_cube() {
        let image = image(catalog_from_xml(
            "http://test.com",
            br#"<krpano showerrors="false" logkey="false">
            <image type="cube" multires="true" tilesize="512" progressive="false" multiresthreshold="-0.3">
                <level download="view" decode="view" tiledimagewidth="1000" tiledimageheight="100">
                    <cube url="http://example.com/%s/%r/%c.jpg"/>
                </level>
            </image>
            </krpano>"#,
        ));
        assert_eq!(image.levels.len(), 6);
        assert_eq!(image.levels[0].size, Some(Vec2d { x: 1000, y: 100 }));
        // Cube faces must remain distinguishable in interactive level pickers.
        let labels: Vec<String> = image
            .levels
            .iter()
            .map(LevelDescriptor::display_label)
            .collect();
        assert!(labels[0].contains("Cube forward"));
        assert!(labels.iter().all(|l| l.contains("1000 x 100 pixels")));
        let unique: std::collections::HashSet<&String> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len(), "labels: {labels:?}");
        assert_eq!(
            format_level_label("Cube", "forward", ""),
            "Krpano Cube forward"
        );
        assert_eq!(
            tile_requests(&image.levels[0], 2),
            vec![
                ("http://example.com/f/1/1.jpg".into(), Vec2d { x: 0, y: 0 }),
                (
                    "http://example.com/f/1/2.jpg".into(),
                    Vec2d { x: 512, y: 0 }
                ),
            ]
        );
    }

    #[test]
    fn test_flat_multires() {
        let image = image(catalog_from_xml(
            "http://test.com",
            br#"<krpano><image><flat url="level=%l x=%0x y=%0y" multires="1,2x3,3x4x3"/></image></krpano>"#,
        ));
        assert_eq!(image.levels.len(), 2);
        assert_eq!(image.levels[1].size, Some(Vec2d { x: 3, y: 4 }));
        assert_eq!(format_level_label("Flat", "", ""), "Krpano Flat");
        assert_eq!(
            tile_requests(&image.levels[1], 2),
            vec![
                (
                    "http://test.com/level=2%20x=01%20y=01".into(),
                    Vec2d { x: 0, y: 0 }
                ),
                (
                    "http://test.com/level=2%20x=01%20y=02".into(),
                    Vec2d { x: 0, y: 3 }
                ),
            ]
        );
    }

    const BELLEGAMBE_XML_URL: &str =
        "https://pba.lille.fr/gigapixels/Gigapixelweb/gigapixels_1515_bellegambe/gigapixels.xml";

    fn assert_bellegambe_levels(levels: &[LevelDescriptor]) {
        let expected_sizes = [
            Vec2d { x: 512, y: 342 },
            Vec2d { x: 768, y: 514 },
            Vec2d { x: 1536, y: 1026 },
            Vec2d { x: 3072, y: 2052 },
            Vec2d { x: 5888, y: 3930 },
            Vec2d { x: 11904, y: 7946 },
            Vec2d { x: 23808, y: 15892 },
            Vec2d { x: 47616, y: 31782 },
            Vec2d { x: 94976, y: 63392 },
        ];
        assert_eq!(levels.len(), expected_sizes.len());
        assert_eq!(
            levels.iter().map(|level| level.size).collect::<Vec<_>>(),
            expected_sizes.into_iter().map(Some).collect::<Vec<_>>()
        );
        let plan = match &levels[7].plan {
            LevelPlan::Known(plan) => plan,
            LevelPlan::Adaptive(_) => unreachable!(),
        };
        assert_eq!(plan.len(), 5859);
        assert_eq!(
            plan.tile(0)
                .unwrap()
                .unwrap()
                .request
                .headers
                .get("Referer")
                .map(String::as_str),
            Some(
                "https://pba.lille.fr/gigapixels/Gigapixelweb/gigapixels_1515_bellegambe/gigapixels.tiles/l8/001/l8_001_001.jpg"
            )
        );
    }

    #[test]
    fn explicit_levels_expand_level_placeholder() {
        let data =
            std::fs::read("testdata/krpano/pba_lille_gigapixels_1515_bellegambe.xml").unwrap();
        assert_bellegambe_levels(&image(catalog_from_xml(BELLEGAMBE_XML_URL, &data)).levels);
    }

    #[test]
    fn explicit_levels_expand_level_placeholder_in_discovery() {
        let data =
            std::fs::read("testdata/krpano/pba_lille_gigapixels_1515_bellegambe.xml").unwrap();
        assert_bellegambe_levels(&image(discover_single_resource(BELLEGAMBE_XML_URL, data)).levels);
    }

    #[test]
    fn test_single_image() {
        let image = image(catalog_from_xml("http://test.com", br#"<krpano><image><flat url="level=%l x=%0x y=%0y" multires="1,2x3,3x4x3"/></image></krpano>"#));
        assert_eq!(image.title, None);
        assert_eq!(image.levels.len(), 2);
    }

    #[test]
    fn test_cube_faces_form_one_image() {
        let image = image(catalog_from_xml("http://test.com", br#"<krpano><image tilesize="512"><level tiledimagewidth="1000" tiledimageheight="100"><cube url="http://example.com/%s/%r/%c.jpg"/></level></image></krpano>"#));
        assert_eq!(image.title, None);
        assert_eq!(image.levels.len(), 6);
        let titles: Vec<_> = image.levels.iter().map(|l| l.title.clone()).collect();
        assert!(!titles.iter().any(Option::is_none));
        let unique: std::collections::HashSet<&Option<String>> = titles.iter().collect();
        assert_eq!(unique.len(), titles.len(), "titles: {titles:?}");
    }

    #[test]
    fn test_multiple_scenes_remain_separate() {
        let data = std::fs::read("testdata/krpano/krpano_scenes.xml").unwrap();
        let titles = catalog_from_xml("http://test.com/scenes.xml", &data)
            .into_entries()
            .into_iter()
            .map(|entry| match entry {
                CatalogEntry::Ready(image) => image.title,
                CatalogEntry::Deferred(_) => unreachable!(),
            })
            .collect::<Vec<_>>();
        assert_eq!(titles, [Some(" Saint Thomas (1618 - 1620) - Diego Velazquez - Museum of Fine Arts, Orleans ( France) scene_Color".into()), Some(" Saint Thomas (1618 - 1620) - Diego Velazquez - Museum of Fine Arts, Orleans ( France) scene_3D".into()), Some(" Saint Thomas (1618 - 1620) - Diego Velazquez - Museum of Fine Arts, Orleans ( France) scene_3Dcolor".into())]);
    }

    #[test]
    fn encrypted_xml_decrypted_without_js() {
        let xml = std::fs::read("testdata/krpano/encrypted/2013-08-09-B/tour.xml").unwrap();
        let expected =
            std::fs::read_to_string("testdata/krpano/encrypted/2013-08-09-B/plaintext.xml")
                .unwrap()
                .replace("\r\n", "\n");
        let plaintext = String::from_utf8(decrypt_xml(&xml, None).unwrap()).unwrap();
        assert_eq!(
            plaintext, expected,
            "decrypted plaintext does not match expected plaintext.xml"
        );
    }

    #[test]
    fn html_script_candidates_prefer_krpano_viewer() {
        let html = r#"<html><head><script src="/assets/jquery.min.js"></script><script src='https://www.googletagmanager.com/gtag/js?id=G-TEST'></script><script data-src="ignored.js" src = "assets/tour.js?cache=1"></script></head></html>"#;
        assert_eq!(
            extract_js_candidates_from_html(html, "http://example.com/pano/index.html")
                .first()
                .map(String::as_str),
            Some("http://example.com/pano/assets/tour.js?cache=1")
        );
    }

    #[test]
    fn sibling_uri_handles_url_and_local_paths() {
        assert_eq!(
            sibling_uri("http://example.com/pano/tour.js", "tour.xml"),
            "http://example.com/pano/tour.xml"
        );
        assert_eq!(
            sibling_uri("http://example.com/pano/", "tour.xml"),
            "http://example.com/pano/tour.xml"
        );
        assert_eq!(
            sibling_uri("/home/user/tour.js", "tour.xml"),
            "/home/user/tour.xml"
        );
        assert_eq!(
            sibling_uri("C:\\foo\\bar\\tour.js", "tour.xml"),
            "C:\\foo\\bar\\tour.xml"
        );
        assert_eq!(
            sibling_uri("\\\\server\\share\\tour.js", "tour.xml"),
            "\\\\server\\share\\tour.xml"
        );
        assert_eq!(sibling_uri("tour.js", "tour.xml"), "tour.xml");
        assert_eq!(
            sibling_uri("https://example.com", "tour.xml"),
            "https://example.com/tour.xml"
        );
        assert_eq!(
            sibling_uri("http://example.com", "tour.js"),
            "http://example.com/tour.js"
        );
        assert_eq!(
            sibling_uri("https://example.com?scene=1", "tour.xml"),
            "https://example.com/tour.xml"
        );
        assert_eq!(
            sibling_uri("https://example.com#section", "tour.xml"),
            "https://example.com/tour.xml"
        );
        assert_eq!(
            sibling_uri("https://example.com/pano/tour.js?cache=1", "tour.xml"),
            "https://example.com/pano/tour.xml"
        );
    }

    #[test]
    fn viewer_js_candidates_derived_from_xml_filename() {
        assert_eq!(
            viewer_js_candidates_for_xml("https://example.com/panos/map_core.xml"),
            vec![
                "https://example.com/panos/map_core.js",
                "https://example.com/panos/tour.js",
                "https://example.com/panos/krpano.js"
            ]
        );
        assert_eq!(
            viewer_js_candidates_for_xml("https://example.com/tour.xml"),
            vec![
                "https://example.com/tour.js",
                "https://example.com/krpano.js"
            ]
        );
        assert_eq!(
            viewer_js_candidates_for_xml("https://example.com/panos/map_core.xml?v=1.2"),
            vec![
                "https://example.com/panos/map_core.js",
                "https://example.com/panos/tour.js",
                "https://example.com/panos/krpano.js"
            ]
        );
    }

    #[test]
    fn extract_xml_from_embedpano_tolerates_whitespace() {
        for html in [
            r#"<script>embedpano({ xml : "panos/tour.xml", target:"pano" });</script>"#,
            r#"embedpano({ "xml": "panos/tour.xml" });"#,
            r#"<script>createPanoViewer({ xml: "panos/tour.xml" });</script>"#,
            r#"<script>EMBEDPANO({ xml: "panos/tour.xml" });</script>"#,
            r#"<script>CreatePanoViewer({ xml: "panos/tour.xml" });</script>"#,
        ] {
            assert_eq!(
                extract_xml_from_embedpano(html),
                Some("panos/tour.xml".into())
            );
        }
        assert_eq!(
            extract_xml_from_embedpano("embedpano({\n xml: \"panos/tour.xml\"\n}\n);"),
            Some("panos/tour.xml".into())
        );
    }

    #[test]
    fn looks_like_krpano_xml_detects_xml_roots() {
        assert!(looks_like_krpano_xml(
            b"<?xml version=\"1.0\"?><krpano></krpano>"
        ));
        assert!(looks_like_krpano_xml(b"<krpano><image></image></krpano>"));
        assert!(looks_like_krpano_xml(
            b"\xef\xbb\xbf<?xml version=\"1.0\"?><krpano/>"
        ));
        assert!(looks_like_krpano_xml(
            b"<?xml version=\"1.0\"?><krpano><action><![CDATA[embedpano();]]></action></krpano>"
        ));
        assert!(!looks_like_krpano_xml(b"<html><body></body></html>"));
        assert!(!looks_like_krpano_xml(b"/* krpano */ function() {}"));
    }

    #[test]
    fn viewer_js_is_detected_before_html_embed_markers() {
        let mut session =
            KrpanoDezoomer.start(&DiscoveryInput::from("https://example.com/krpano.js"));
        let _ = session.advance(DiscoveryEvent::Start).unwrap();
        let step = session
            .advance(DiscoveryEvent::Resource(&ResourceOutcome::Response(
                crate::core::ResourceResponse {
                    id: crate::core::RequestId(0),
                    bytes: b"function embedpano(opts) { /* krpano viewer */ }".to_vec()
                },
            )))
            .unwrap();
        assert!(
            matches!(step, DiscoveryStep::Need(ResourceRequest { request }) if request.uri == "https://example.com/tour.xml")
        );
    }

    #[test]
    fn old_create_pano_viewer_js_is_detected_as_viewer_js() {
        let mut session =
            KrpanoDezoomer.start(&DiscoveryInput::from("https://example.com/viewer.js"));
        let _ = session.advance(DiscoveryEvent::Start).unwrap();
        let step = session
            .advance(DiscoveryEvent::Resource(&ResourceOutcome::Response(
                crate::core::ResourceResponse {
                    id: crate::core::RequestId(0),
                    bytes: b"function createPanoViewer(opts) { return buildViewer(opts); }"
                        .to_vec()
                },
            )))
            .unwrap();
        assert!(
            matches!(step, DiscoveryStep::Need(ResourceRequest { request }) if request.uri == "https://example.com/tour.xml")
        );
    }

    #[test]
    fn looks_like_krpano_html_requires_krpano_evidence() {
        for html in [
            b"<html><script>embedpano({xml:'tour.xml'})</script></html>".as_slice(),
            b"<HTML><BODY><SCRIPT>EMBEDPANO({xml:'tour.xml'})</SCRIPT></BODY></HTML>".as_slice(),
            b"<script>createPanoViewer({xml:'tour.xml'});</script>".as_slice(),
            b"<html><script src='krpano.js'></script></html>".as_slice(),
            b"<html><script src='tour.js'></script></html>".as_slice(),
            b"<HTML><SCRIPT SRC='tour.js'></SCRIPT></HTML>".as_slice(),
        ] {
            assert!(looks_like_krpano_html(html));
        }
        for html in [
            b"<html><script src='jquery.min.js'></script></html>".as_slice(),
            b"<HTML><SCRIPT src='analytics.js'></SCRIPT></HTML>".as_slice(),
            b"<html><body>Hello</body></html>".as_slice(),
        ] {
            assert!(!looks_like_krpano_html(html));
        }
    }
}
