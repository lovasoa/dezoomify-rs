use std::sync::Arc;

use custom_error::custom_error;
use itertools::Itertools;
use log::warn;

use krpano_metadata::{KrpanoMetadata, TemplateString, TemplateStringPart, XY};

use crate::dezoomer::*;
use crate::krpano::krpano_metadata::{ImageInfo, LevelDesc};
use crate::network::resolve_relative;
use encrypted::decrypt_xml;
use log::{debug, warn};

mod encrypted;
mod krpano_metadata;

#[derive(Debug)]
pub struct KrpanoZoomableImage {
    zoom_levels: ZoomLevels,
    title: Option<String>,
}

impl KrpanoZoomableImage {
    pub fn new(zoom_levels: ZoomLevels, title: Option<String>) -> Self {
        Self { zoom_levels, title }
    }
}

impl ZoomableImageWithLevels for KrpanoZoomableImage {
    fn into_zoom_levels(self: Box<Self>) -> Result<ZoomLevels, DezoomerError> {
        Ok(self.zoom_levels)
    }

    fn title(&self) -> Option<String> {
        self.title.clone()
    }
}

/// A dezoomer for krpano images
/// See https://krpano.com/docu/xml/#top
#[derive(Default)]
pub struct KrpanoDezoomer {
    /// State machine for the NeedsData resolution chain.
    state: ResolveState,
}

/// Where we are in the HTML → JS → XML → (decrypt JS) resolution chain.
#[derive(Default)]
enum ResolveState {
    #[default]
    None,
    /// HTML loaded; XML URL known from embedpano.  Need the viewer JS.
    NeedJs { xml_uri: String },
    /// Viewer JS loaded; need the XML config to proceed.
    /// Carries the JS so it can be reused if the XML is encrypted.
    NeedXml {
        xml_uri: String,
        viewer_js: Vec<u8>,
    },
    /// Encrypted XML is pending; need the viewer JS to decrypt it.
    /// (Used when the entry point is XML directly, not HTML.)
    NeedJsToDecrypt {
        xml_uri: String,
        xml_contents: Vec<u8>,
    },
}

impl Dezoomer for KrpanoDezoomer {
    fn name(&self) -> &'static str {
        "krpano"
    }

    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        self.handle_input(data, load_from_properties)
    }

    fn dezoomer_result(&mut self, data: &DezoomerInput) -> Result<DezoomerResult, DezoomerError> {
        self.handle_input(data, |uri, contents| {
            let images = load_images_from_properties(uri, contents)?;
            Ok(dezoomer_result_from_images(images))
        })
    }
}

impl KrpanoDezoomer {
    /// Navigate the HTML → JS → XML → (decrypt if encrypted) resolution chain.
    fn handle_input<T>(
        &mut self,
        data: &DezoomerInput,
        parse: impl FnOnce(&str, &[u8]) -> Result<T, DezoomerError>,
    ) -> Result<T, DezoomerError> {
        let DezoomerInputWithContents { uri, contents } = data.with_contents()?;
        debug!("krpano handle_input: uri={uri}, content_len={}", contents.len());

        // --- State machine dispatch ---

        // HTML → JS: we know the XML URL, the current call is the viewer JS.
        // Save the JS and request the XML.
        if matches!(&self.state, ResolveState::NeedJs { .. }) {
            let xml_uri = match &self.state {
                ResolveState::NeedJs { xml_uri } => xml_uri.clone(),
                _ => unreachable!(),
            };
            debug!("krpano state=NeedJs → got JS ({} bytes), requesting XML: {xml_uri}", contents.len());
            let viewer_js = contents.to_vec();
            self.state = ResolveState::NeedXml {
                xml_uri: xml_uri.clone(),
                viewer_js,
            };
            return Err(DezoomerError::NeedsData { uri: xml_uri });
        }

        // JS → XML: we have the viewer JS; the current call is the XML.
        // If encrypted, decrypt with the saved JS.  Otherwise parse directly.
        if let ResolveState::NeedXml {
            xml_uri,
            viewer_js,
        } = &self.state
        {
            let (xml_uri, viewer_js) = (xml_uri.clone(), viewer_js.clone());
            self.state = ResolveState::None;
            debug!("krpano state=NeedXml → got content ({} bytes), xml_uri={xml_uri}", contents.len());

            if encrypted::is_encrypted_xml(contents) {
                debug!("krpano: XML is encrypted, decrypting with saved viewer JS ({} bytes)", viewer_js.len());
                let decrypted = decrypt_xml(contents, Some(&viewer_js))?;
                debug!("krpano: decrypted XML = {} bytes", decrypted.len());
                return parse(&xml_uri, &decrypted);
            }
            debug!("krpano: XML is plain, parsing directly");
            return parse(&xml_uri, contents);
        }

        // Encrypted XML entry point: need viewer JS to decrypt.
        if let ResolveState::NeedJsToDecrypt {
            xml_uri,
            xml_contents,
        } = &self.state
        {
            let (xml_uri, xml_contents) = (xml_uri.clone(), xml_contents.clone());
            self.state = ResolveState::None;

            // Use raw contents as viewer JS (handles packed viewers).
            let viewer_js = extract_viewer_js(contents).unwrap_or_else(|| contents.to_vec());
            match decrypt_xml(&xml_contents, Some(&viewer_js)) {
                Ok(decrypted) => return parse(&xml_uri, &decrypted),
                Err(_) => {
                    self.state = ResolveState::NeedJsToDecrypt {
                        xml_uri,
                        xml_contents,
                    };
                }
            }
        }

        // --- Content-type detection (fresh entry) ---

        if looks_like_html(contents) {
            let html = String::from_utf8_lossy(contents);
            let js_uri = extract_js_from_html(&html, uri);
            let xml_uri = extract_xml_from_embedpano(&html)
                .map(|rel| resolve_relative(uri, &rel))
                .unwrap_or_else(|| sibling_uri(uri, "tour.xml"));

            if let Some(js_uri) = js_uri {
                self.state = ResolveState::NeedJs { xml_uri };
                return Err(DezoomerError::NeedsData { uri: js_uri });
            }
            return Err(DezoomerError::NeedsData { uri: xml_uri });
        }

        if looks_like_viewer_js(contents) {
            let xml_uri = sibling_uri(uri, "tour.xml");
            return Err(DezoomerError::NeedsData { uri: xml_uri });
        }

        if encrypted::is_encrypted_xml(contents) {
            self.state = ResolveState::NeedJsToDecrypt {
                xml_uri: uri.to_string(),
                xml_contents: contents.to_vec(),
            };
            let js_uri = sibling_uri(uri, "tour.js");
            return Err(DezoomerError::NeedsData { uri: js_uri });
        }

        // Plain (non-encrypted) krpano XML — parse directly.
        parse(uri, contents)
    }
}

/// True if the content looks like an HTML page.
fn looks_like_html(contents: &[u8]) -> bool {
    let text = String::from_utf8_lossy(contents);
    text.contains("<html") || text.contains("<script") || text.contains("embedpano(")
}

/// True if the content looks like a krpano viewer JavaScript file.
fn looks_like_viewer_js(contents: &[u8]) -> bool {
    let text = String::from_utf8_lossy(contents);
    (text.starts_with("function ") || text.contains("eval("))
        && (text.contains("krpano") || text.contains("loadpano") || text.contains("embedhtml5"))
}

/// Try to extract a viewer JS URL from an HTML page.
/// Returns the first `<script src="...">` that is not inline.
fn extract_js_from_html(html: &str, html_uri: &str) -> Option<String> {
    // Find any <script src="..."> tag.
    for line in html.lines() {
        let lower = line.to_lowercase();
        if lower.contains("<script") && lower.contains("src=") {
            if let Some(src) = extract_src_attr(line) {
                // Skip common non-viewer scripts.
                if !src.ends_with(".js") {
                    continue;
                }
                return Some(resolve_relative(html_uri, &src));
            }
        }
    }
    None
}

/// Extract the XML URL from an `embedpano({xml:"..."})` call in an HTML page.
fn extract_xml_from_embedpano(html: &str) -> Option<String> {
    // Look for xml:"..." or xml:'...' inside embedpano({...})
    let start = html.find("embedpano(")?;
    let body = &html[start..];
    let end = body.find("})")?;
    let params = &body[..end + 2];
    // Extract xml:"..." or xml:'...'
    if let Some(xml_start) = params.find("xml:") {
        let rest = &params[xml_start + 4..];
        // Skip optional whitespace after xml:
        let rest = rest.trim_start();
        let quote = rest.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let inner = &rest[1..];
        let xml_end = inner.find(quote)?;
        return Some(inner[..xml_end].to_string());
    }
    None
}

/// Extract the `src` attribute value from a <script> tag.
fn extract_src_attr(line: &str) -> Option<String> {
    let start = line.find("src=")?;
    let rest = &line[start + 4..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let end = rest[1..].find(quote)?;
    Some(rest[1..=end].to_string())
}

/// Extract viewer JS from a data block — the content might be the JS itself,
/// or an HTML wrapper.  Returns the JS bytes if found.
fn extract_viewer_js(contents: &[u8]) -> Option<Vec<u8>> {
    if looks_like_viewer_js(contents) {
        return Some(contents.to_vec());
    }
    // If wrapped in HTML, look for inline <script> blocks.
    let text = String::from_utf8_lossy(contents);
    if let Some(start) = text.find("<script>") {
        let body = &text[start + 8..];
        if let Some(end) = body.find("</script>") {
            let js = body[..end].trim();
            if looks_like_viewer_js(js.as_bytes()) {
                return Some(js.as_bytes().to_vec());
            }
        }
    }
    None
}

/// Replace the last path component of `uri` with `filename`.
fn sibling_uri(uri: &str, filename: &str) -> String {
    if let Some((dir, _)) = uri.rsplit_once('/') {
        format!("{dir}/{filename}")
    } else {
        filename.to_string()
    }
}

custom_error! {pub KrpanoError
    XmlError{source: serde_xml_rs::Error} = "Unable to parse the krpano xml file: {source}",
}

impl From<encrypted::EncryptedKrpanoError> for DezoomerError {
    fn from(err: encrypted::EncryptedKrpanoError) -> Self {
        DezoomerError::Other { source: err.into() }
    }
}

impl From<KrpanoError> for DezoomerError {
    fn from(err: KrpanoError) -> Self {
        DezoomerError::Other { source: err.into() }
    }
}

fn load_from_properties(url: &str, contents: &[u8]) -> Result<ZoomLevels, DezoomerError> {
    let decrypted;
    let contents = if encrypted::is_encrypted_xml(contents) {
        decrypted = decrypt_xml(contents, None)?;
        decrypted.as_slice()
    } else {
        contents
    };
    let image_properties: KrpanoMetadata =
        serde_xml_rs::from_reader(contents).map_err(KrpanoError::from)?;
    let base_url = &Arc::from(url);
    let title: &Arc<str> = &Arc::from(image_properties.get_title().unwrap_or(""));
    Ok(image_properties
        .into_image_iter()
        .flat_map(move |ImageInfo { image, name }| {
            let root_tile_size = image.tilesize.map(Vec2d::square);
            let base_index = image.baseindex;
            image.level.into_iter().flat_map(move |level| {
                let name = Arc::clone(&name);
                level
                    .level_descriptions(None)
                    .into_iter()
                    .flat_map(move |level_desc| {
                        let name = Arc::clone(&name);
                        level_desc
                            .map_err(|err| warn!("bad krpano level: {err}"))
                            .into_iter()
                            .flat_map(
                                move |LevelDesc {
                                          name: shape_name,
                                          size,
                                          tilesize,
                                          url,
                                          level_index,
                                      }| {
                                    let level = level_index + base_index as usize;
                                    let name = Arc::clone(&name);
                                    url.all_sides(level).flat_map(move |(side_name, template)| {
                                        let base_url = Arc::clone(base_url);
                                        let title = Arc::clone(title);
                                        let name = Arc::clone(&name);
                                        tilesize.or(root_tile_size).map(|tile_size| Level {
                                            base_url,
                                            size,
                                            tile_size,
                                            base_index,
                                            template,
                                            shape_name,
                                            side_name,
                                            name,
                                            title,
                                        })
                                    })
                                },
                            )
                    })
            })
        })
        .into_zoom_levels())
}

fn load_images_from_properties(
    url: &str,
    contents: &[u8],
) -> Result<Vec<Box<dyn ZoomableImageWithLevels>>, DezoomerError> {
    let decrypted;
    let contents = if encrypted::is_encrypted_xml(contents) {
        decrypted = decrypt_xml(contents, None)?;
        decrypted.as_slice()
    } else {
        contents
    };
    let image_properties: KrpanoMetadata =
        serde_xml_rs::from_reader(contents).map_err(KrpanoError::from)?;
    let base_url = Arc::from(url);
    let global_title = image_properties.get_title().unwrap_or("").to_string();

    let images: Vec<Box<dyn ZoomableImageWithLevels>> = image_properties
        .into_image_iter()
        .map(|ImageInfo { image, name }| {
            let root_tile_size = image.tilesize.map(Vec2d::square);
            let base_index = image.baseindex;
            let base_url = Arc::clone(&base_url);
            let global_title_for_levels = Arc::from(global_title.as_str());
            let name_for_levels = Arc::clone(&name);

            let levels: ZoomLevels = image
                .level
                .into_iter()
                .flat_map(move |level| {
                    let name = Arc::clone(&name_for_levels);
                    let base_url = Arc::clone(&base_url);
                    let global_title = Arc::clone(&global_title_for_levels);
                    level
                        .level_descriptions(None)
                        .into_iter()
                        .flat_map(move |level_desc| {
                            let name = Arc::clone(&name);
                            let base_url = Arc::clone(&base_url);
                            let global_title = Arc::clone(&global_title);
                            level_desc
                                .map_err(|err| warn!("bad krpano level: {err}"))
                                .into_iter()
                                .flat_map(
                                    move |LevelDesc {
                                              name: shape_name,
                                              size,
                                              tilesize,
                                              url,
                                              level_index,
                                          }| {
                                        let level = level_index + base_index as usize;
                                        let name = Arc::clone(&name);
                                        let base_url = Arc::clone(&base_url);
                                        let global_title = Arc::clone(&global_title);
                                        url.all_sides(level).flat_map(
                                            move |(side_name, template)| {
                                                let base_url = Arc::clone(&base_url);
                                                let name = Arc::clone(&name);
                                                let global_title = Arc::clone(&global_title);
                                                tilesize.or(root_tile_size).map(|tile_size| Level {
                                                    base_url,
                                                    size,
                                                    tile_size,
                                                    base_index,
                                                    template,
                                                    shape_name,
                                                    side_name,
                                                    name: Arc::clone(&name),
                                                    title: Arc::clone(&global_title),
                                                })
                                            },
                                        )
                                    },
                                )
                        })
                })
                .into_zoom_levels();

            let image_title = if name.is_empty() && global_title.is_empty() {
                None
            } else {
                let title = [global_title.as_str(), name.as_ref()]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                Some(title)
            };

            Box::new(KrpanoZoomableImage::new(levels, image_title))
                as Box<dyn ZoomableImageWithLevels>
        })
        .collect();

    Ok(images)
}

#[derive(PartialEq, Eq)]
struct Level {
    base_url: Arc<str>,
    size: Vec2d,
    tile_size: Vec2d,
    base_index: u32,
    template: TemplateString<XY>,
    shape_name: &'static str,
    side_name: &'static str,
    name: Arc<str>,
    title: Arc<str>,
}

impl TilesRect for Level {
    fn size(&self) -> Vec2d {
        self.size
    }

    fn tile_size(&self) -> Vec2d {
        self.tile_size
    }

    fn tile_url(&self, Vec2d { x, y }: Vec2d) -> String {
        use std::fmt::Write;
        let mut result = String::new();
        for part in self.template.0.iter() {
            match part {
                TemplateStringPart::Literal(s) => result += s,
                TemplateStringPart::Variable { padding, variable } => {
                    write!(
                        result,
                        "{value:0padding$}",
                        value = self.base_index
                            + match variable {
                                XY::X => x,
                                XY::Y => y,
                            },
                        padding = *padding
                    )
                    .unwrap();
                }
            }
        }
        resolve_relative(&self.base_url, &result)
    }

    fn title(&self) -> Option<String> {
        if self.title.is_empty() && self.name.is_empty() {
            None
        } else {
            let title = [self.title.as_ref(), self.name.as_ref()].join(" ");
            Some(title)
        }
    }

    fn tile_ref(&self, pos: Vec2d) -> TileReference {
        TileReference {
            url: self.tile_url(pos),
            position: self.tile_size() * pos,
        }
    }
}

impl std::fmt::Debug for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let parts = ["Krpano", self.shape_name, self.side_name, &self.name];
        write!(f, "{}", parts.iter().filter(|s| !s.is_empty()).join(" "))
    }
}

#[test]
fn test_cube() {
    let mut levels = load_from_properties(
        "http://test.com",
        r#"<krpano showerrors="false" logkey="false">
        <image type="cube" multires="true" tilesize="512" progressive="false" multiresthreshold="-0.3">
            <level download="view" decode="view" tiledimagewidth="1000" tiledimageheight="100">
                <cube url="http://example.com/%s/%r/%c.jpg"/>
            </level>
        </image>
        </krpano>"#.as_bytes(),
    ).unwrap();
    assert_eq!(levels.len(), 6);
    assert_eq!(levels[0].size_hint(), Some(Vec2d { x: 1000, y: 100 }));
    assert_eq!(format!("{:?}", levels[0]), "Krpano Cube forward");
    assert_eq!(
        levels[0].next_tiles(None),
        vec![
            TileReference {
                url: "http://example.com/f/1/1.jpg".to_string(),
                position: Vec2d { x: 0, y: 0 }
            },
            TileReference {
                url: "http://example.com/f/1/2.jpg".to_string(),
                position: Vec2d { x: 512, y: 0 }
            }
        ]
    );
}

#[test]
fn test_flat_multires() {
    let mut levels = load_from_properties(
        "http://test.com",
        r#"<krpano>
        <image>
            <flat url="level=%l x=%0x y=%0y" multires="1,2x3,3x4x3"/>
        </image>
        </krpano>"#
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(levels.len(), 2);
    assert_eq!(levels[1].size_hint(), Some(Vec2d { x: 3, y: 4 }));
    assert_eq!(format!("{:?}", levels[0]), "Krpano Flat");
    assert_eq!(
        levels[1].next_tiles(None),
        vec![
            TileReference {
                url: "http://test.com/level=2%20x=01%20y=01".to_string(),
                position: Vec2d { x: 0, y: 0 }
            },
            TileReference {
                url: "http://test.com/level=2%20x=01%20y=02".to_string(),
                position: Vec2d { x: 0, y: 3 }
            }
        ]
    );
}

#[test]
fn test_dezoomer_result_single_image() {
    let mut dezoomer = KrpanoDezoomer::default();
    let data = r#"<krpano>
        <image>
            <flat url="level=%l x=%0x y=%0y" multires="1,2x3,3x4x3"/>
        </image>
        </krpano>"#
        .as_bytes();

    let input = DezoomerInput {
        uri: "http://test.com".to_string(),
        contents: PageContents::Success(data.to_vec()),
    };

    let result = dezoomer.dezoomer_result(&input).unwrap();
    assert_eq!(result.len(), 1);

    if let ZoomableImage::Image(ref image) = result[0] {
        assert_eq!(image.title(), None);
    } else {
        panic!("Expected ZoomableImage::Image");
    }
}

#[test]
fn test_dezoomer_result_cube_faces() {
    let mut dezoomer = KrpanoDezoomer::default();
    let data = r#"<krpano showerrors="false" logkey="false">
        <image type="cube" multires="true" tilesize="512" progressive="false" multiresthreshold="-0.3">
            <level download="view" decode="view" tiledimagewidth="1000" tiledimageheight="100">
                <cube url="http://example.com/%s/%r/%c.jpg"/>
            </level>
        </image>
        </krpano>"#.as_bytes();

    let input = DezoomerInput {
        uri: "http://test.com".to_string(),
        contents: PageContents::Success(data.to_vec()),
    };

    let result = dezoomer.dezoomer_result(&input).unwrap();
    assert_eq!(result.len(), 1);

    if let ZoomableImage::Image(ref image) = result[0] {
        assert_eq!(image.title(), None);
    } else {
        panic!("Expected ZoomableImage::Image");
    }
}

#[test]
fn test_dezoomer_result_multiple_scenes() {
    let mut dezoomer = KrpanoDezoomer::default();
    let data = std::fs::read("testdata/krpano/krpano_scenes.xml").unwrap();

    let input = DezoomerInput {
        uri: "http://test.com/scenes.xml".to_string(),
        contents: PageContents::Success(data),
    };

    let result = dezoomer.dezoomer_result(&input).unwrap();
    assert_eq!(result.len(), 3);

    let titles: Vec<Option<String>> = result
        .iter()
        .map(|zoomable_img| {
            if let ZoomableImage::Image(image) = zoomable_img {
                image.title()
            } else {
                panic!("Expected ZoomableImage::Image");
            }
        })
        .collect();
    assert!(titles.contains(&Some(" Saint Thomas (1618 - 1620) - Diego Velazquez - Museum of Fine Arts, Orleans ( France) scene_Color".to_string())));
    assert!(titles.contains(&Some(" Saint Thomas (1618 - 1620) - Diego Velazquez - Museum of Fine Arts, Orleans ( France) scene_3D".to_string())));
    assert!(titles.contains(&Some(" Saint Thomas (1618 - 1620) - Diego Velazquez - Museum of Fine Arts, Orleans ( France) scene_3Dcolor".to_string())));
}

#[test]
fn encrypted_xml_triggers_needs_data() {
    let mut dezoomer = KrpanoDezoomer::default();
    let xml = std::fs::read("testdata/krpano/encrypted/2018-04-04/tour.xml").unwrap();

    let input = DezoomerInput {
        uri: "http://example.com/tour.xml".to_string(),
        contents: PageContents::Success(xml),
    };

    let result = dezoomer.zoom_levels(&input);
    match result {
        Err(DezoomerError::NeedsData { uri }) => {
            assert_eq!(uri, "http://example.com/tour.js");
        }
        other => panic!("expected NeedsData, got {other:?}"),
    }
}

#[test]
fn encrypted_xml_second_call_decrypts() {
    let mut dezoomer = KrpanoDezoomer::default();
    let xml = std::fs::read("testdata/krpano/encrypted/2018-04-04/tour.xml").unwrap();
    let js = std::fs::read("testdata/krpano/encrypted/2018-04-04/tour.js").unwrap();

    // First call: encrypted XML → NeedsData for tour.js.
    let input_xml = DezoomerInput {
        uri: "http://example.com/tour.xml".to_string(),
        contents: PageContents::Success(xml),
    };
    let needs = dezoomer.zoom_levels(&input_xml).unwrap_err();
    assert!(matches!(needs, DezoomerError::NeedsData { .. }));

    // Second call: viewer JS (fetched by caller) → decrypt and parse.
    let input_js = DezoomerInput {
        uri: "http://example.com/tour.js".to_string(),
        contents: PageContents::Success(js),
    };
    let result = dezoomer.zoom_levels(&input_js);
    match result {
        Ok(_levels) => { /* success */ }
        Err(e) => panic!("second call failed: {e}"),
    }
}
