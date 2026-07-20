use std::sync::Arc;

use custom_error::custom_error;
use log::{debug, warn};

use tile_info::ImageInfo;

use crate::dezoomer::*;
use crate::iiif::tile_info::TileSizeFormat;
use crate::json_utils::all_json;
use crate::max_size_in_rect;

pub mod manifest_types;
pub mod tile_info;

/// Dezoomer for the International Image Interoperability Framework.
/// See https://iiif.io/
#[derive(Default)]
pub struct IIIF;

/// Represents a single IIIF image with metadata from a manifest or info.json
#[derive(Debug)]
pub struct IIIFZoomableImage {
    zoom_levels: ZoomLevels,
    title: Option<String>,
}

impl IIIFZoomableImage {
    pub fn new(zoom_levels: ZoomLevels, title: Option<String>) -> Self {
        IIIFZoomableImage { zoom_levels, title }
    }
}

impl ZoomableImageWithLevels for IIIFZoomableImage {
    fn into_zoom_levels(self: Box<Self>) -> Result<ZoomLevels, DezoomerError> {
        Ok(self.zoom_levels)
    }

    fn title(&self) -> Option<String> {
        self.title.clone()
    }
}

/// Determines the best title for an image from IIIF manifest metadata
pub fn determine_title(image_info: &manifest_types::ExtractedImageInfo) -> Option<String> {
    let mut parts = Vec::new();

    if let Some(manifest_label) = &image_info.manifest_label {
        parts.push(manifest_label.as_str());
    }

    if let Some(metadata_title) = &image_info.metadata_title
        && !parts.contains(&metadata_title.as_str())
    {
        parts.push(metadata_title.as_str());
    }

    if let Some(canvas_label) = &image_info.canvas_label
        && !parts.contains(&canvas_label.as_str())
    {
        parts.push(canvas_label.as_str());
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" - "))
    }
}

custom_error! {pub IIIFError
    JsonError{source: serde_json::Error} = "Invalid IIIF info.json file: {source}",
    ManifestParseError{description: String} = "Could not parse IIIF manifest: {description}",
}

impl From<IIIFError> for DezoomerError {
    fn from(err: IIIFError) -> Self {
        DezoomerError::Other { source: err.into() }
    }
}

impl Dezoomer for IIIF {
    fn name(&self) -> &'static str {
        "iiif"
    }

    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        let with_contents = data.with_contents()?;
        let contents = with_contents.contents;
        let uri = with_contents.uri;
        Ok(zoom_levels(uri, contents)?)
    }

    fn dezoomer_result(&mut self, data: &DezoomerInput) -> Result<DezoomerResult, DezoomerError> {
        let with_contents = data.with_contents()?;
        let contents = with_contents.contents;
        let uri = with_contents.uri;

        // First, try to determine what type of IIIF content this is by doing a quick parse
        // to check the "type" field without generating warnings
        if let Ok(quick_check) = serde_json::from_slice::<serde_json::Value>(contents)
            && let Some(type_value) = quick_check.get("type").or_else(|| quick_check.get("@type"))
            && let Some(type_str) = type_value.as_str()
        {
            match type_str {
                "ImageService2" | "ImageService3" | "iiif:ImageProfile" => {
                    // This is clearly an Image Service info.json, try parsing it directly
                    let levels = zoom_levels(uri, contents)?;
                    let image = IIIFZoomableImage::new(levels, None);
                    return Ok(dezoomer_result_from_single_image(image));
                }
                "Manifest" | "sc:Manifest" => {
                    // This is clearly a manifest, try parsing it as such
                    match parse_iiif_manifest_from_bytes(contents, uri) {
                        Ok(image_infos) if !image_infos.is_empty() => {
                            return Ok(dezoomer_result_from_manifest_image_infos(image_infos));
                        }
                        Ok(_) => {
                            // Empty image_infos, fall through to heuristic approach
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                _ => {
                    // Unknown type, fall through to heuristic detection below
                }
            }
        }

        // If type detection didn't work or type is unknown, use heuristic approach
        // Check if URL suggests it's an info.json file
        if uri.ends_with("/info.json") {
            // Likely an Image Service, try parsing as info.json first
            match zoom_levels(uri, contents) {
                Ok(levels) => {
                    let image = IIIFZoomableImage::new(levels, None);
                    return Ok(dezoomer_result_from_single_image(image));
                }
                Err(_) => {
                    // Fall through to try as manifest
                }
            }
        }

        // Try to parse as IIIF manifest
        match parse_iiif_manifest_from_bytes(contents, uri) {
            Ok(image_infos) if !image_infos.is_empty() => {
                // Successfully parsed as manifest with images
                Ok(dezoomer_result_from_manifest_image_infos(image_infos))
            }
            _ => {
                // Not a manifest or failed to parse as manifest, try as info.json
                match zoom_levels(uri, contents) {
                    Ok(levels) => {
                        let image = IIIFZoomableImage::new(levels, None);
                        Ok(dezoomer_result_from_single_image(image))
                    }
                    Err(e) => Err(e.into()),
                }
            }
        }
    }
}

fn dezoomer_result_from_manifest_image_infos(
    image_infos: Vec<manifest_types::ExtractedImageInfo>,
) -> DezoomerResult {
    let image_urls: Vec<ZoomableImageUrl> = image_infos
        .into_iter()
        .map(|image_info| {
            let title = determine_title(&image_info);
            ZoomableImageUrl {
                url: image_info.image_uri,
                title,
            }
        })
        .collect();

    dezoomer_result_from_urls(image_urls)
}

fn zoom_levels(url: &str, raw_info: &[u8]) -> Result<ZoomLevels, IIIFError> {
    match serde_json::from_slice(raw_info) {
        Ok(info) => Ok(zoom_levels_from_info(url, info)),
        Err(e) => {
            // Due to the very fault-tolerant way we parse iiif manifests, a single javascript
            // object with a 'width' and a 'height' field is enough to be detected as an IIIF level
            // See https://github.com/lovasoa/dezoomify-rs/issues/80
            let levels: Vec<ZoomLevel> = all_json::<ImageInfo>(raw_info)
                .filter(|info| {
                    let keep = info.has_distinctive_iiif_properties();
                    if keep {
                        debug!(
                            "keeping image info {info:?} because it has distinctive IIIF properties"
                        )
                    } else {
                        debug!("dropping level {info:?}")
                    }
                    keep
                })
                .flat_map(|info| zoom_levels_from_info(url, info))
                .collect();
            if levels.is_empty() {
                Err(e.into())
            } else {
                debug!(
                    "No normal info.json parsing failed ({}), \
                but {} inline json5 zoom level(s) were found.",
                    e,
                    levels.len()
                );
                Ok(levels)
            }
        }
    }
}

fn zoom_levels_from_info(url: &str, mut image_info: ImageInfo) -> ZoomLevels {
    image_info.remove_test_id();
    image_info.resolve_relative_urls(url);
    let img = Arc::new(image_info);
    let tiles = img.tiles();
    let base_url = &Arc::from(url.replace("/info.json", ""));

    tiles
        .iter()
        .flat_map(|tile_info| {
            let tile_size = tile_info.size();
            let quality = Arc::from(img.best_quality());
            let format = Arc::from(img.best_format());
            let size_format = img.preferred_size_format();
            debug!(
                "Chose the following image parameters: tile_size=({tile_size}) quality={quality} format={format}"
            );
            let page_info = &img; // Required to allow the move
            tile_info.scale_factors.iter().map(move |&scale_factor| {
                let zoom_level = IIIFZoomLevel {
                    scale_factor,
                    tile_size,
                    page_info: Arc::clone(page_info),
                    base_url: Arc::clone(base_url),
                    quality: Arc::clone(&quality),
                    format: Arc::clone(&format),
                    size_format,
                };
                debug!("Found zoom level {zoom_level:?}: page_info: {page_info:?}, tile_size: {tile_size:?}, scale_factor: {scale_factor}, base_url: {base_url}, quality: {quality}, format: {format}, size_format: {size_format:?}");
                zoom_level
            })
        })
        .into_zoom_levels()
}

struct IIIFZoomLevel {
    scale_factor: u32,
    tile_size: Vec2d,
    page_info: Arc<ImageInfo>,
    base_url: Arc<str>,
    quality: Arc<str>,
    format: Arc<str>,
    size_format: TileSizeFormat,
}

impl TilesRect for IIIFZoomLevel {
    fn size(&self) -> Vec2d {
        self.page_info.size() / self.scale_factor
    }

    fn tile_size(&self) -> Vec2d {
        self.tile_size
    }

    fn tile_url(&self, col_and_row_pos: Vec2d) -> String {
        let scaled_tile_size = self.tile_size * self.scale_factor;
        let xy_pos = col_and_row_pos * scaled_tile_size;
        let scaled_tile_size = max_size_in_rect(xy_pos, scaled_tile_size, self.page_info.size());
        let tile_size = scaled_tile_size / self.scale_factor;
        format!(
            "{base}/{x},{y},{img_w},{img_h}/{tile_size}/{rotation}/{quality}.{format}",
            base = self
                .page_info
                .id
                .as_deref()
                .unwrap_or_else(|| self.base_url.as_ref()),
            x = xy_pos.x,
            y = xy_pos.y,
            img_w = scaled_tile_size.x,
            img_h = scaled_tile_size.y,
            tile_size = TileSizeFormatter {
                w: tile_size.x,
                h: tile_size.y,
                format: self.size_format
            },
            rotation = 0,
            quality = self.quality,
            format = self.format,
        )
    }

    fn scale_factor_hint(&self) -> Option<u32> {
        Some(self.scale_factor)
    }
}

struct TileSizeFormatter {
    w: u32,
    h: u32,
    format: TileSizeFormat,
}

impl std::fmt::Display for TileSizeFormatter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.format {
            TileSizeFormat::WidthHeight => write!(f, "{},{}", self.w, self.h),
            TileSizeFormat::Width => write!(f, "{},", self.w),
        }
    }
}

impl std::fmt::Debug for IIIFZoomLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let name = self
            .page_info
            .id
            .as_deref()
            .unwrap_or_else(|| self.base_url.as_ref())
            .split('/')
            .next_back()
            .and_then(|s: &str| {
                let s = s.trim();
                if s.is_empty() { None } else { Some(s) }
            })
            .unwrap_or("IIIF Image");
        write!(f, "{name}")
    }
}

/// Parses a IIIF Presentation API Manifest from byte content.
///
/// # Arguments
/// * `bytes` - The raw byte content of the manifest file.
/// * `manifest_url` - The original URL from which the manifest was fetched. This is crucial
///   for resolving any relative URLs found within the manifest.
///
/// # Returns
/// A `Result` containing a vector of `ExtractedImageInfo` if successful,
/// or an `IIIFError` if parsing fails or the content is not a valid manifest.
pub fn parse_iiif_manifest_from_bytes(
    bytes: &[u8],
    manifest_url: &str,
) -> Result<Vec<manifest_types::ExtractedImageInfo>, IIIFError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| IIIFError::JsonError { source: e })?;

    if is_legacy_presentation_manifest(&value) {
        parse_legacy_presentation_manifest(bytes, manifest_url)
    } else if is_presentation3_manifest(&value) {
        parse_presentation3_manifest(bytes, manifest_url)
    } else {
        parse_unknown_manifest(bytes, manifest_url)
    }
}

fn is_presentation3_manifest(value: &serde_json::Value) -> bool {
    manifest_type(value) == Some("Manifest")
        || json_context_contains(value, "iiif.io/api/presentation/3")
}

fn is_legacy_presentation_manifest(value: &serde_json::Value) -> bool {
    manifest_type(value) == Some("sc:Manifest")
        || json_context_contains(value, "iiif.io/api/presentation/2")
        || json_context_contains(value, "shared-canvas.org/ns/context")
}

fn manifest_type(value: &serde_json::Value) -> Option<&str> {
    value
        .get("type")
        .or_else(|| value.get("@type"))
        .and_then(|type_value| type_value.as_str())
}

fn json_context_contains(value: &serde_json::Value, needle: &str) -> bool {
    match value.get("@context") {
        Some(serde_json::Value::String(context)) => context.contains(needle),
        Some(serde_json::Value::Array(contexts)) => contexts.iter().any(|context| {
            context
                .as_str()
                .is_some_and(|context| context.contains(needle))
        }),
        _ => false,
    }
}

fn parse_presentation3_manifest(
    bytes: &[u8],
    manifest_url: &str,
) -> Result<Vec<manifest_types::ExtractedImageInfo>, IIIFError> {
    let manifest: manifest_types::Manifest =
        serde_json::from_slice(bytes).map_err(|e| IIIFError::JsonError { source: e })?;

    if manifest.manifest_type != "Manifest" {
        // Don't warn for known IIIF Image Service types, as these are valid but not manifests
        if !matches!(
            manifest.manifest_type.as_str(),
            "ImageService2" | "ImageService3" | "iiif:ImageProfile"
        ) {
            // While we could be more lenient, the Presentation API spec says this should be "Manifest".
            // If it's something else, it's likely not what we expect, or a different IIIF spec.
            warn!(
                "Attempted to parse IIIF manifest from {} but 'type' field was '{}' instead of 'Manifest'. Proceeding, but this may indicate an incorrect file type.",
                manifest_url, manifest.manifest_type
            );
        }
    }

    Ok(manifest.extract_image_infos(manifest_url))
}

fn parse_legacy_presentation_manifest(
    bytes: &[u8],
    manifest_url: &str,
) -> Result<Vec<manifest_types::ExtractedImageInfo>, IIIFError> {
    let manifest: manifest_types::LegacyManifest =
        serde_json::from_slice(bytes).map_err(|e| IIIFError::JsonError { source: e })?;

    Ok(manifest.extract_image_infos(manifest_url))
}

fn parse_unknown_manifest(
    bytes: &[u8],
    manifest_url: &str,
) -> Result<Vec<manifest_types::ExtractedImageInfo>, IIIFError> {
    match parse_presentation3_manifest(bytes, manifest_url) {
        Ok(image_infos) if !image_infos.is_empty() => Ok(image_infos),
        Ok(_) => match parse_legacy_presentation_manifest(bytes, manifest_url) {
            Ok(image_infos) if !image_infos.is_empty() => Ok(image_infos),
            _ => Ok(Vec::new()),
        },
        Err(v3_error) => match parse_legacy_presentation_manifest(bytes, manifest_url) {
            Ok(image_infos) if !image_infos.is_empty() => Ok(image_infos),
            _ => Err(v3_error),
        },
    }
}

#[test]
fn test_tiles() {
    let data = br#"{
            .split('/')
            .next_back()
            .and_then(|s: &str| {
                let s = s.trim();
                if s.is_empty() { None } else { Some(s) }
            })
            .unwrap_or("IIIF Image");
        write!(f, "{name}")
    }
}

#[test]
fn test_tiles() {
    let data = br#"{
      "@context" : "http://iiif.io/api/image/2/context.json",
      "@id" : "http://www.asmilano.it/fast/iipsrv.fcgi?IIIF=/opt/divenire/files/./tifs/05/36/536765.tif",
      "protocol" : "http://iiif.io/api/image",
      "width" : 15001,
      "height" : 48002,
      "tiles" : [
         { "width" : 512, "height" : 512, "scaleFactors" : [ 1, 2, 4, 8, 16, 32, 64, 128 ] }
      ],
      "profile" : [
         "http://iiif.io/api/image/2/level1.json",
         { "formats" : [ "jpg" ],
           "qualities" : [ "native","color","gray" ],
           "supports" : ["regionByPct","sizeByForcedWh","sizeByWh","sizeAboveFull","rotationBy90s","mirroring","gray"] }
      ]
    }"#;
    let mut levels = zoom_levels("test.com", data).unwrap();
    let tiles: Vec<String> = levels[6]
        .next_tiles(None)
        .into_iter()
        .map(|t| t.url)
        .collect();
    assert_eq!(
        tiles,
        vec![
            "http://www.asmilano.it/fast/iipsrv.fcgi?IIIF=/opt/divenire/files/./tifs/05/36/536765.tif/0,0,15001,32768/234,512/0/default.jpg",
            "http://www.asmilano.it/fast/iipsrv.fcgi?IIIF=/opt/divenire/files/./tifs/05/36/536765.tif/0,32768,15001,15234/234,238/0/default.jpg",
        ]
    )
}

#[test]
fn test_tiles_max_area_filter() {
    // Predefined tile size (1024x1024) is over maxArea (262144 = 512x512).
    // See https://github.com/lovasoa/dezoomify-rs/issues/107#issuecomment-862225501
    let data = br#"{
      "width" : 1024,
      "height" : 1024,
      "tiles" : [{ "width" : 1024, "scaleFactors" : [ 1 ] }],
      "profile" :  [ { "maxArea": 262144 } ]
    }"#;
    let mut levels = zoom_levels("http://ophir.dev/info.json", data).unwrap();
    let tiles: Vec<String> = levels[0]
        .next_tiles(None)
        .into_iter()
        .map(|t| t.url)
        .collect();
    assert_eq!(
        tiles,
        vec![
            "http://ophir.dev/0,0,512,512/512,512/0/default.jpg",
            "http://ophir.dev/512,0,512,512/512,512/0/default.jpg",
            "http://ophir.dev/0,512,512,512/512,512/0/default.jpg",
            "http://ophir.dev/512,512,512,512/512,512/0/default.jpg",
        ]
    )
}

#[test]
fn test_missing_id() {
    let data = br#"{
      "width" : 600,
      "height" : 350
    }"#;
    let mut levels = zoom_levels("http://test.com/info.json", data).unwrap();
    let tiles: Vec<String> = levels[0]
        .next_tiles(None)
        .into_iter()
        .map(|t| t.url)
        .collect();
    assert_eq!(
        tiles,
        vec![
            "http://test.com/0,0,512,350/512,350/0/default.jpg",
            "http://test.com/512,0,88,350/88,350/0/default.jpg"
        ]
    )
}

#[test]
fn test_false_positive() {
    let data = br#"
    var mainImage={
        type:       "zoomifytileservice",
        width:      62596,
        height:     38467,
        tilesUrl:   "./ORIONFINAL/"
    };
    "#;
    let res = zoom_levels("https://orion2020v5b.spaceforeverybody.com/", data);
    assert!(
        res.is_err(),
        "openseadragon zoomify image should not be misdetected"
    );
}

#[test]
fn test_qualities() {
    let data = br#"{
        "@context": "http://library.stanford.edu/iiif/image-api/1.1/context.json",
        "@id": "https://images.britishart.yale.edu/iiif/fd470c3e-ead0-4878-ac97-d63295753f82",
        "tile_height": 1024,
        "tile_width": 1024,
        "width": 5156,
        "height": 3816,
        "profile": "http://library.stanford.edu/iiif/image-api/1.1/compliance.html#level0",
        "qualities": [ "native", "color", "bitonal", "gray", "zorglub" ],
        "formats" : [ "png", "zorglub" ],
        "scale_factors": [ 10 ]
    }"#;
    let mut levels = zoom_levels("test.com", data).unwrap();
    let level = &mut levels[0];
    assert_eq!(level.size_hint(), Some(Vec2d { x: 515, y: 381 })); // 5156/10, 3816/10
    let tiles: Vec<String> = level.next_tiles(None).into_iter().map(|t| t.url).collect();
    assert_eq!(
        tiles,
        vec![
            "https://images.britishart.yale.edu/iiif/fd470c3e-ead0-4878-ac97-d63295753f82/0,0,5156,3816/515,381/0/native.png", // tile_width and tile_height are not used from profile here but from image_info.tile_w/h
        ]
    )
}

#[cfg(test)]
mod manifest_parsing_tests {
    use super::*;
    use crate::iiif::manifest_types::ExtractedImageInfo;

    fn legacy_manifest_data() -> &'static [u8] {
        r#"{
          "@context":"http://iiif.io/api/presentation/2/context.json","@type":"sc:Manifest",
          "label":"Legacy Book","sequences":[{"canvases":[{"label":"Page 1","images":[{"resource":{
            "@type":"dctypes:Image","@id":"https://example.com/iiif/page1/full/843,/0/default.jpg",
            "service":{"@id":"https://example.com/iiif/page1"}
          }}]}]}]
        }"#
        .as_bytes()
    }

    #[test]
    fn test_parse_simple_manifest_from_bytes() {
        let manifest_url = "https://example.com/manifest.json";
        let json_data = r#"
        {
          "@context": "http://iiif.io/api/presentation/3/context.json",
          "id": "https://example.org/iiif/book1/manifest",
          "type": "Manifest",
          "label": { "en": [ "Book Example" ] },
          "items": [
            {
              "id": "canvas1",
              "type": "Canvas",
              "label": { "en": [ "Page 1" ] },
              "items": [
                {
                  "id": "anno_page1",
                  "type": "AnnotationPage",
                  "items": [
                    {
                      "id": "anno1",
                      "type": "Annotation",
                      "motivation": "painting",
                      "body": {
                        "id": "http://example.images/page1_img_direct.jpg",
                        "type": "Image",
                        "service": [
                          {
                            "id": "svc/page1_svc", 
                            "type": "ImageService2"
                          }
                        ]
                      }
                    }
                  ]
                }
              ]
            }
          ]
        }
        "#;
        let result = parse_iiif_manifest_from_bytes(json_data.as_bytes(), manifest_url);
        assert!(result.is_ok());
        let infos = result.unwrap();
        assert_eq!(infos.len(), 1);
        assert_eq!(
            infos[0],
            ExtractedImageInfo {
                image_uri: "https://example.com/svc/page1_svc/info.json".to_string(), // Resolved
                manifest_label: Some("Book Example".to_string()),
                metadata_title: None,
                canvas_label: Some("Page 1".to_string()),
                canvas_index: 0,
            }
        );
    }

    #[test]
    fn test_parse_manifest_with_relative_paths_from_bytes() {
        let manifest_url = "https://library.example.edu/collection/item123/manifest.json";
        let json_data = r#"
        {
          "id": "relative-manifest",
          "type": "Manifest",
          "label": { "en": ["RelPath Test"] },
          "items": [
            {
              "id": "c1", "type": "Canvas", "label": {"en": ["C1 Rel Svc"]},
              "items": [{"id": "ap1", "type": "AnnotationPage", "items": [{"id": "a1", "type": "Annotation", "motivation": "painting",
                  "body": { "id": "../images/image1.jpg", "type": "Image", "service": [{"id": "../services/image1_svc", "type": "ImageService3"}]}
              }]}]
            },
            {
              "id": "c2", "type": "Canvas", "label": {"en": ["C2 Abs Path Svc"]},
              "items": [{"id": "ap2", "type": "AnnotationPage", "items": [{"id": "a2", "type": "Annotation", "motivation": "painting",
                  "body": { "id": "/img/abs_image2.png", "type": "Image", "service": [{"id": "/iiif-services/abs_image2_svc", "type": "ImageService2"}]}
              }]}]
            },
            {
              "id": "c3", "type": "Canvas", "label": {"en": ["C3 Direct Rel Img"]},
              "items": [{"id": "ap3", "type": "AnnotationPage", "items": [{"id": "a3", "type": "Annotation", "motivation": "painting",
                  "body": { "id": "images/cover_art.jpeg", "type": "Image" }
              }]}]
            }
          ]
        }
        "#;

        let result = parse_iiif_manifest_from_bytes(json_data.as_bytes(), manifest_url);
        assert!(result.is_ok(), "Parsing failed: {:?}", result.err());
        let infos = result.unwrap();
        assert_eq!(infos.len(), 3);

        assert_eq!(
            infos[0].image_uri,
            "https://library.example.edu/collection/services/image1_svc/info.json"
        );
        assert_eq!(infos[0].manifest_label, Some("RelPath Test".to_string()));
        assert_eq!(infos[0].canvas_label, Some("C1 Rel Svc".to_string()));

        assert_eq!(
            infos[1].image_uri,
            "https://library.example.edu/iiif-services/abs_image2_svc/info.json"
        );
        assert_eq!(infos[1].canvas_label, Some("C2 Abs Path Svc".to_string()));

        assert_eq!(
            infos[2].image_uri,
            "https://library.example.edu/collection/item123/images/cover_art.jpeg"
        );
        assert_eq!(infos[2].canvas_label, Some("C3 Direct Rel Img".to_string()));
    }

    #[test]
    fn test_parse_legacy_manifest_from_bytes() {
        let infos = parse_iiif_manifest_from_bytes(
            legacy_manifest_data(),
            "https://api.artic.edu/api/v1/artworks/103887/manifest.json",
        )
        .unwrap();

        assert_eq!(infos.len(), 1);
        assert_eq!(
            infos[0].image_uri,
            "https://example.com/iiif/page1/info.json"
        );
    }

    #[test]
    fn test_parse_invalid_json_manifest() {
        let manifest_url = "https://example.com/invalid.json";
        let json_data = r#"{ "id": "test", "type": "Manifest", items: [ -- broken json -- ] }"#;
        let result = parse_iiif_manifest_from_bytes(json_data.as_bytes(), manifest_url);
        assert!(result.is_err());
        match result.err().unwrap() {
            IIIFError::JsonError { .. } => {} // Expected
            e => panic!("Expected JsonError, got {:?}", e),
        }
    }

    #[test]
    fn test_parse_json_not_a_manifest_type() {
        let manifest_url = "https://example.com/not_a_manifest.json";
        let json_data = r#"{ "id": "test", "type": "NotAManifest", "items": [] }"#;
        // This should parse fine based on struct leniency, but we log a warning.
        // The function itself should succeed if the structure is parsable into Manifest.
        let result = parse_iiif_manifest_from_bytes(json_data.as_bytes(), manifest_url);
        assert!(result.is_ok());
        // The `extract_image_infos` method would then be called on this.
        // For a more strict check, one might add an explicit error if manifest.manifest_type != "Manifest".
        // The current implementation logs a warning and proceeds.
        let infos = result.unwrap();
        assert_eq!(infos.len(), 0); // No items that would yield images.
    }

    #[test]
    fn test_dezoomer_result_with_manifest() {
        let mut dezoomer = IIIF;
        let manifest_data = r#"
        {
          "@context": "http://iiif.io/api/presentation/3/context.json",
          "id": "https://example.org/iiif/book1/manifest",
          "type": "Manifest",
          "label": { "en": [ "Test Book" ] },
          "items": [
            {
              "id": "canvas1",
              "type": "Canvas",
              "label": { "en": [ "Page 1" ] },
              "items": [
                {
                  "id": "anno_page1",
                  "type": "AnnotationPage",
                  "items": [
                    {
                      "id": "anno1",
                      "type": "Annotation",
                      "motivation": "painting",
                      "body": {
                        "id": "image.jpg",
                        "type": "Image",
                        "service": [
                          {
                            "id": "https://example.com/iiif/page1",
                            "type": "ImageService3"
                          }
                        ]
                      }
                    }
                  ]
                }
              ]
            }
          ]
        }
        "#
        .as_bytes();

        let input = DezoomerInput {
            uri: "https://example.com/manifest.json".to_string(),
            contents: PageContents::Success(manifest_data.to_vec()),
        };

        let result = dezoomer.dezoomer_result(&input).unwrap();
        assert_eq!(result.len(), 1);

        if let ZoomableImage::ImageUrl(ref url) = result[0] {
            assert_eq!(url.url, "https://example.com/iiif/page1/info.json");
            assert_eq!(url.title, Some("Test Book - Page 1".to_string()));
        } else {
            panic!("Expected ZoomableImage::ImageUrl");
        }
    }

    #[test]
    fn test_dezoomer_result_with_legacy_manifest() {
        let mut dezoomer = IIIF;
        let input = DezoomerInput {
            uri: "https://example.com/manifest.json".to_string(),
            contents: PageContents::Success(legacy_manifest_data().to_vec()),
        };

        let result = dezoomer.dezoomer_result(&input).unwrap();
        assert_eq!(result.len(), 1);

        if let ZoomableImage::ImageUrl(ref url) = result[0] {
            assert_eq!(url.url, "https://example.com/iiif/page1/info.json");
            assert_eq!(url.title, Some("Legacy Book - Page 1".to_string()));
        } else {
            panic!("Expected ZoomableImage::ImageUrl");
        }
    }

    #[test]
    fn test_dezoomer_result_with_info_json() {
        let mut dezoomer = IIIF;
        let info_data = r#"{
          "@context" : "http://iiif.io/api/image/2/context.json",
          "@id" : "https://example.com/image",
          "protocol" : "http://iiif.io/api/image",
          "width" : 1000,
          "height" : 1500,
          "tiles" : [
             { "width" : 512, "height" : 512, "scaleFactors" : [ 1, 2, 4 ] }
          ]
        }"#
        .as_bytes();

        let input = DezoomerInput {
            uri: "https://example.com/image/info.json".to_string(),
            contents: PageContents::Success(info_data.to_vec()),
        };

        let result = dezoomer.dezoomer_result(&input).unwrap();
        assert_eq!(result.len(), 1);

        if let ZoomableImage::Image(ref image) = result[0] {
            assert_eq!(image.title(), None);
        } else {
            panic!("Expected ZoomableImage::Image");
        }
    }
}
