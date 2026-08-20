use serde::Deserialize;
use std::collections::HashMap;

use crate::core::resolve_relative;

// Helper for potentially multilingual labels
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(untagged)]
#[derive(Default)]
pub enum IiifLabel {
    String(String),
    Map(HashMap<String, Vec<String>>),
    #[default]
    None, // Represents JSON null or a deliberately empty label
}

impl IiifLabel {
    /// Returns the English label if present, otherwise the first label found for any language,
    /// or the string itself if it's a simple string label. Returns None if the label is empty or explicitly None.
    #[must_use]
    pub fn get_english_or_first(&self) -> Option<String> {
        match self {
            IiifLabel::String(s) => (!s.is_empty()).then(|| s.clone()),
            IiifLabel::Map(map) => {
                if let Some(en_labels) = map.get("en")
                    && let Some(first_en) = en_labels.first().filter(|s| !s.is_empty())
                {
                    return Some(first_en.clone());
                }
                // Fallback to the first non-empty label in the first language found
                map.values()
                    .find_map(|labels| labels.first().filter(|s| !s.is_empty()).cloned())
            }
            IiifLabel::None => None,
        }
    }
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct MetadataEntry {
    #[serde(default)]
    pub label: IiifLabel,
    #[serde(default)]
    pub value: IiifLabel,
}

impl MetadataEntry {
    /// Get the metadata title if this entry is a title field
    #[must_use]
    pub fn get_title(&self) -> Option<String> {
        if let Some(label) = self.label.get_english_or_first()
            && label.to_lowercase() == "title"
        {
            return self.value.get_english_or_first();
        }
        None
    }
}

// Default implementation to handle missing labels gracefully via #[serde(default)]

#[allow(clippy::struct_field_names)]
#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct Manifest {
    #[serde(default, rename = "@context", skip_deserializing)]
    pub context: Option<String>, // Can be string or array of strings
    #[serde(default)] // If ID is missing, we might use the fetch URL as a fallback later
    pub id: String,
    #[serde(default, rename = "type")]
    pub manifest_type: String, // Should be "Manifest"
    #[serde(default)]
    pub label: IiifLabel,
    #[serde(default)]
    pub items: Vec<Canvas>,
    #[serde(default)]
    pub metadata: Option<Vec<MetadataEntry>>,
    // Other potentially useful fields:
    // pub summary: Option<IiifLabel>,
    // pub thumbnail: Option<Vec<Thumbnail>>,
}

#[allow(clippy::struct_field_names)]
#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct Canvas {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub canvas_type: String, // Should be "Canvas"
    #[serde(default)]
    pub label: IiifLabel,
    #[serde(default)]
    pub items: Vec<AnnotationPage>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[allow(clippy::struct_field_names)]
#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct AnnotationPage {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub annotation_page_type: String, // Should be "AnnotationPage"
    #[serde(default)]
    pub items: Vec<Annotation>,
}

#[allow(clippy::struct_field_names)]
#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct Annotation {
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub annotation_type: String, // Should be "Annotation"
    pub motivation: Option<String>, // We are interested in "painting"
    #[serde(default)] // Body could be missing or not an image
    pub body: AnnotationBody,
    // target: Option<Target>, // Target is usually the canvas; can be string or object
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(untagged)] // AnnotationBody can be a single ImageBody or potentially an array or other types.
// For "painting" motivation, it's typically a single ImageBody object.
// If it can be an array, this definition needs adjustment.
// For now, assuming it's a single object or can be missing (handled by Option in usage or default)
pub enum AnnotationBody {
    Image(ImageBody),
    #[default]
    EmptyOrUnsupported, // Catch-all for missing body or types we don't handle
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct ImageBody {
    #[serde(default)]
    pub id: String, // This can be the direct image URL or the base URI for an Image Service
    #[serde(default, rename = "type")]
    pub image_type: String, // Should be "Image"
    pub format: Option<String>, // e.g., "image/jpeg"
    #[serde(default)]
    pub service: Vec<ImageService>, // Array of services, can be empty
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct ImageService {
    #[serde(alias = "@id")] // IIIF Image API 2 uses "@id"
    #[serde(default)]
    pub id: String, // IIIF Image API 3 uses "id"
    #[serde(alias = "@type")] // IIIF Image API 2 uses "@type"
    #[serde(default, rename = "type")]
    pub service_type: String, // e.g. "ImageService2", "ImageService3"
    #[serde(default, skip_deserializing)]
    pub profile: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct LegacyManifest {
    #[serde(default)]
    pub label: IiifLabel,
    #[serde(default)]
    pub sequences: Vec<LegacySequence>,
    #[serde(default)]
    pub metadata: Option<Vec<MetadataEntry>>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct LegacySequence {
    #[serde(default)]
    pub canvases: Vec<LegacyCanvas>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct LegacyCanvas {
    #[serde(default)]
    pub label: IiifLabel,
    #[serde(default)]
    pub images: Vec<LegacyAnnotation>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct LegacyAnnotation {
    #[serde(default)]
    pub resource: LegacyImageResource,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct LegacyImageResource {
    #[serde(default, alias = "@id")]
    pub id: String,
    #[serde(default, rename = "type", alias = "@type")]
    pub resource_type: String,
    #[serde(default)]
    pub service: LegacyImageServices,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(untagged)]
pub enum LegacyImageServices {
    Single(ImageService),
    Multiple(Vec<ImageService>),
    #[default]
    None,
}

impl LegacyImageServices {
    fn as_slice(&self) -> &[ImageService] {
        match self {
            LegacyImageServices::Single(service) => std::slice::from_ref(service),
            LegacyImageServices::Multiple(services) => services.as_slice(),
            LegacyImageServices::None => &[],
        }
    }
}

/// Holds information extracted from a manifest for a single image to be dezoomified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedImageInfo {
    /// The URI to be dezoomified (either an `info.json` or a direct image link).
    pub image_uri: String,
    /// The label of the manifest, if available.
    pub manifest_label: Option<String>,
    /// The title from metadata, if available.
    pub metadata_title: Option<String>,
    /// The label of the canvas this image belongs to, if available.
    pub canvas_label: Option<String>,
    /// The 0-based index of the canvas in the manifest's `items` array.
    pub canvas_index: usize,
}

fn image_service_info_uri(manifest_url: &str, service_id: &str) -> String {
    let mut resolved_uri = resolve_relative(manifest_url, service_id);
    if !resolved_uri.ends_with("/info.json") {
        if !resolved_uri.ends_with('/') {
            resolved_uri.push('/');
        }
        resolved_uri.push_str("info.json");
    }
    resolved_uri
}

fn choose_image_service_uri(services: &[ImageService], manifest_url: &str) -> Option<String> {
    let service = services
        .iter()
        .find(|s| s.service_type == "ImageService3" && !s.id.is_empty())
        .or_else(|| {
            services
                .iter()
                .find(|s| s.service_type == "ImageService2" && !s.id.is_empty())
        })
        .or_else(|| {
            services
                .iter()
                .find(|s| s.service_type.contains("ImageService") && !s.id.is_empty())
        })
        .or_else(|| services.iter().find(|s| !s.id.is_empty()))?;

    Some(image_service_info_uri(manifest_url, &service.id))
}

impl Manifest {
    /// Get the title from metadata if available
    #[must_use]
    pub fn get_metadata_title(&self) -> Option<String> {
        self.metadata
            .as_ref()?
            .iter()
            .find_map(MetadataEntry::get_title)
    }

    /// Extracts all relevant image URIs (info.json or direct image links) from the manifest.
    ///
    /// It traverses Canvases, `AnnotationPages`, and Annotations to find "painting"
    /// motivations where the body is an Image. It prioritizes Image Services
    /// (`ImageService2` or `ImageService3`) and constructs `info.json` URIs.
    /// If no service is found, it uses the direct image `id` from the annotation body.
    /// All extracted URIs are resolved relative to `manifest_url`.
    #[must_use]
    pub fn extract_image_infos(&self, manifest_url: &str) -> Vec<ExtractedImageInfo> {
        let mut infos = Vec::new();
        let manifest_label = self.label.get_english_or_first();
        let metadata_title = self.get_metadata_title();

        for (canvas_index, canvas) in self.items.iter().enumerate() {
            // We expect "Canvas" type, but proceed even if it's different,
            // as long as it contains annotation pages with painting annotations.
            // if canvas.canvas_type != "Canvas" { continue; }

            let canvas_label = canvas.label.get_english_or_first();

            for annotation_page in &canvas.items {
                // Expect "AnnotationPage", but proceed if structure matches.
                // if annotation_page.annotation_page_type != "AnnotationPage" { continue; }

                for annotation in &annotation_page.items {
                    if let AnnotationBody::Image(image_body) = &annotation.body {
                        // Expect "Image" type for the body, but rely on service presence.
                        // if image_body.image_type != "Image" { continue; }

                        let final_image_uri = choose_image_service_uri(
                            &image_body.service,
                            manifest_url,
                        )
                        .or_else(|| {
                            if !image_body.id.is_empty() && image_body.image_type == "Image" {
                                Some(resolve_relative(manifest_url, &image_body.id))
                            } else {
                                None
                            }
                        });

                        if let Some(uri_to_add) = final_image_uri {
                            infos.push(ExtractedImageInfo {
                                image_uri: uri_to_add,
                                manifest_label: manifest_label.clone(),
                                metadata_title: metadata_title.clone(),
                                canvas_label: canvas_label.clone(),
                                canvas_index,
                            });
                        }
                    }
                }
            }
        }
        infos
    }
}

impl LegacyManifest {
    /// Get the title from metadata if available
    #[must_use]
    pub fn get_metadata_title(&self) -> Option<String> {
        self.metadata
            .as_ref()?
            .iter()
            .find_map(MetadataEntry::get_title)
    }

    /// Extracts image URIs from IIIF Presentation v1/v2 manifests.
    #[must_use]
    pub fn extract_image_infos(&self, manifest_url: &str) -> Vec<ExtractedImageInfo> {
        let mut infos = Vec::new();
        let manifest_label = self.label.get_english_or_first();
        let metadata_title = self.get_metadata_title();

        for (canvas_index, canvas) in self
            .sequences
            .iter()
            .flat_map(|sequence| sequence.canvases.iter())
            .enumerate()
        {
            let canvas_label = canvas.label.get_english_or_first();

            for annotation in &canvas.images {
                let resource = &annotation.resource;
                let final_image_uri =
                    choose_image_service_uri(resource.service.as_slice(), manifest_url).or_else(
                        || {
                            if !resource.id.is_empty()
                                && (resource.resource_type.is_empty()
                                    || resource.resource_type.ends_with("Image"))
                            {
                                Some(resolve_relative(manifest_url, &resource.id))
                            } else {
                                None
                            }
                        },
                    );

                if let Some(uri_to_add) = final_image_uri {
                    infos.push(ExtractedImageInfo {
                        image_uri: uri_to_add,
                        manifest_label: manifest_label.clone(),
                        metadata_title: metadata_title.clone(),
                        canvas_label: canvas_label.clone(),
                        canvas_index,
                    });
                }
            }
        }

        infos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_label_extraction() {
        let label_str = IiifLabel::String("Hello".to_string());
        assert_eq!(label_str.get_english_or_first(), Some("Hello".to_string()));

        let label_empty_str = IiifLabel::String(String::new());
        assert_eq!(label_empty_str.get_english_or_first(), None);

        let mut map_en = HashMap::new();
        map_en.insert("en".to_string(), vec!["World".to_string()]);
        map_en.insert("fr".to_string(), vec!["Monde".to_string()]);
        let label_map_en = IiifLabel::Map(map_en);
        assert_eq!(
            label_map_en.get_english_or_first(),
            Some("World".to_string())
        );

        let mut map_en_empty_val = HashMap::new();
        map_en_empty_val.insert("en".to_string(), vec![String::new()]);
        map_en_empty_val.insert("fr".to_string(), vec!["Monde".to_string()]);
        let label_map_en_empty = IiifLabel::Map(map_en_empty_val);
        assert_eq!(
            label_map_en_empty.get_english_or_first(),
            Some("Monde".to_string())
        );

        let mut map_fr_only = HashMap::new();
        map_fr_only.insert("fr".to_string(), vec!["Monde".to_string()]);
        let label_map_fr_only = IiifLabel::Map(map_fr_only);
        assert_eq!(
            label_map_fr_only.get_english_or_first(),
            Some("Monde".to_string())
        );

        let label_none = IiifLabel::None;
        assert_eq!(label_none.get_english_or_first(), None);

        let empty_map = HashMap::new();
        let label_empty_map = IiifLabel::Map(empty_map);
        assert_eq!(label_empty_map.get_english_or_first(), None);
    }

    #[test]
    fn test_deserialize_basic_manifest_and_extract_infojson() {
        let json_data = r#"
        {
          "@context": "http://iiif.io/api/presentation/3/context.json",
          "id": "https://example.org/iiif/book1/manifest",
          "type": "Manifest",
          "label": { "en": [ "Book 1" ] },
          "items": [
            {
              "id": "https://example.org/iiif/book1/canvas/p1",
              "type": "Canvas",
              "label": { "en": [ "Page 1" ] },
              "width": 1000,
              "height": 1500,
              "items": [
                {
                  "id": "https://example.org/iiif/book1/canvas/p1/page",
                  "type": "AnnotationPage",
                  "items": [
                    {
                      "id": "https://example.org/iiif/book1/canvas/p1/page/image",
                      "type": "Annotation",
                      "motivation": "painting",
                      "body": {
                        "id": "https://example.org/iiif/book1/page1_img/full/max/0/default.jpg",
                        "type": "Image",
                        "format": "image/jpeg",
                        "service": [
                          {
                            "id": "https://example.org/iiif/book1/page1_svc",
                            "type": "ImageService2",
                            "profile": "level1"
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
        let manifest: Manifest = serde_json::from_str(json_data).unwrap();
        assert_eq!(manifest.manifest_type, "Manifest");
        assert_eq!(
            manifest.label.get_english_or_first(),
            Some("Book 1".to_string())
        );
        assert_eq!(manifest.items.len(), 1);

        let infos = manifest.extract_image_infos("https://example.org/iiif/book1/manifest");
        assert_eq!(infos.len(), 1);
        let info = &infos[0];
        assert_eq!(
            info.image_uri,
            "https://example.org/iiif/book1/page1_svc/info.json"
        );
        assert_eq!(info.manifest_label, Some("Book 1".to_string()));
        assert_eq!(info.canvas_label, Some("Page 1".to_string()));
        assert_eq!(info.canvas_index, 0);
    }

    #[test]
    fn test_extract_direct_image_uri_no_service() {
        let json_data = r#"
        {
          "id": "https://example.org/manifest-no-service",
          "type": "Manifest",
          "items": [
            {
              "id": "https://example.org/canvas/c1",
              "type": "Canvas",
              "items": [
                {
                  "id": "ap1",
                  "type": "AnnotationPage",
                  "items": [
                    {
                      "id": "a1",
                      "type": "Annotation",
                      "motivation": "painting",
                      "body": {
                        "id": "https://example.org/images/direct_image.jpg",
                        "type": "Image",
                        "format": "image/jpeg"
                      }
                    }
                  ]
                }
              ]
            }
          ]
        }
        "#;

        let manifest: Manifest = serde_json::from_str(json_data).unwrap();
        let infos = manifest.extract_image_infos("https://example.org/manifest-no-service");
        assert_eq!(infos.len(), 1);
        assert_eq!(
            infos[0].image_uri,
            "https://example.org/images/direct_image.jpg"
        );
        assert_eq!(infos[0].manifest_label, None);
        assert_eq!(infos[0].canvas_label, None);
        assert_eq!(infos[0].canvas_index, 0);
    }

    #[test]
    fn test_prioritize_image_service_3_over_2() {
        let json_data = r#"
        {
          "id": "manifest-svc-priority", "type": "Manifest",
          "items": [{ "id": "c1", "type": "Canvas", "items": [{ "id": "ap1", "type": "AnnotationPage", "items": [
            { "id": "a1", "type": "Annotation", "motivation": "painting",
              "body": { "id": "img.jpg", "type": "Image",
                "service": [
                  { "id": "https://example.org/svc2", "type": "ImageService2" },
                  { "id": "https://example.org/svc3", "type": "ImageService3" }
                ]
              }
            }]
          }]}]
        }
        "#;

        let manifest: Manifest = serde_json::from_str(json_data).unwrap();
        let infos = manifest.extract_image_infos("https://example.org/"); // Base URL for resolving "svc3"
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].image_uri, "https://example.org/svc3/info.json");
    }

    #[test]
    fn test_service_id_already_has_info_json() {
        let json_data = r#"
        {
          "id": "manifest-info-json-in-id", "type": "Manifest",
          "items": [{ "id": "c1", "type": "Canvas", "items": [{ "id": "ap1", "type": "AnnotationPage", "items": [
            { "id": "a1", "type": "Annotation", "motivation": "painting",
              "body": { "id": "irrelevant.jpg", "type": "Image",
                "service": [ { "id": "https://example.org/iiif/img_already_info/info.json", "type": "ImageService3" } ]
              }
            }]
          }]}]
        }
        "#;

        let manifest: Manifest = serde_json::from_str(json_data).unwrap();
        // Base URL doesn't matter here as the service ID is absolute and complete
        let infos = manifest.extract_image_infos("https://unused.example.com/");
        assert_eq!(infos.len(), 1);
        assert_eq!(
            infos[0].image_uri,
            "https://example.org/iiif/img_already_info/info.json"
        );
    }

    #[test]
    fn test_multiple_canvases_and_images() {
        let json_data = r#"
        {
          "id": "multi-manifest", "type": "Manifest", "label": {"en": ["Multi"]},
          "items": [
            {
              "id": "c1", "type": "Canvas", "label": {"en": ["Canvas 1"]},
              "items": [{ "id": "ap1", "type": "AnnotationPage", "items": [{
                  "id": "a1", "type": "Annotation", "motivation": "painting",
                  "body": { "id": "img1.jpg", "type": "Image", "service": [{"id": "svc1", "type": "ImageService2"}] }
              }]}]
            },
            {
              "id": "c2", "type": "Canvas", "label": {"en": ["Canvas 2"]},
              "items": [{ "id": "ap2", "type": "AnnotationPage", "items": [
                  { "id": "a2.1", "type": "Annotation", "motivation": "painting",
                    "body": { "id": "img2.1.jpg", "type": "Image", "service": [{"id": "svc2.1", "type": "ImageService3"}] }},
                  { "id": "a2.2", "type": "Annotation", "motivation": "painting",
                    "body": { "id": "img2.2.png", "type": "Image" }}
              ]}]
            }
          ]
        }
        "#;

        let manifest: Manifest = serde_json::from_str(json_data).unwrap();
        let manifest_base_url = "https://example.com/base/";
        let infos = manifest.extract_image_infos(manifest_base_url);
        assert_eq!(infos.len(), 3);

        assert_eq!(
            infos[0].image_uri,
            "https://example.com/base/svc1/info.json"
        );
        assert_eq!(infos[0].manifest_label, Some("Multi".to_string()));
        assert_eq!(infos[0].canvas_label, Some("Canvas 1".to_string()));
        assert_eq!(infos[0].canvas_index, 0);

        assert_eq!(
            infos[1].image_uri,
            "https://example.com/base/svc2.1/info.json"
        );
        assert_eq!(infos[1].manifest_label, Some("Multi".to_string()));
        assert_eq!(infos[1].canvas_label, Some("Canvas 2".to_string()));
        assert_eq!(infos[1].canvas_index, 1);

        assert_eq!(infos[2].image_uri, "https://example.com/base/img2.2.png");
        assert_eq!(infos[2].manifest_label, Some("Multi".to_string()));
        assert_eq!(infos[2].canvas_label, Some("Canvas 2".to_string()));
        assert_eq!(infos[2].canvas_index, 1);
    }

    #[test]
    fn test_real_world_example_bl_digirati_simplified() {
        let json_data = r#"
        {
            "@context": "http://iiif.io/api/presentation/3/context.json",
            "id": "https://bl.digirati.io/iiif/ark:/81055/man_10000006.0x000001",
            "type": "Manifest",
            "label": { "en": [ "Cotton MS Nero D IV" ] },
            "items": [ {
                "id": "...", "type": "Canvas", "label": { "en": [ "Front cover" ] },
                "items": [ {
                    "id": "...", "type": "AnnotationPage",
                    "items": [ {
                        "id": "...", "type": "Annotation", "motivation": "painting",
                        "body": {
                            "id": ".../default.jpg", "type": "Image",
                            "service": [
                                { "@id": "https://bl.digirati.io/images/ark:/81055/81055/man_10000006.0x000002", "@type": "ImageService2" },
                                { "id": "https://dlcs.bl.digirati.io/iiif-img/v3/.../man_10000006.0x000002", "type": "ImageService3" }
                            ]
                        }
                    } ]
                } ]
            } ]
        }
        "#;

        let manifest: Manifest = serde_json::from_str(json_data).expect("Failed to parse manifest");
        let infos = manifest
            .extract_image_infos("https://bl.digirati.io/iiif/ark:/81055/man_10000006.0x000001");
        assert_eq!(infos.len(), 1);
        // Should pick ImageService3
        assert_eq!(
            infos[0].image_uri,
            "https://dlcs.bl.digirati.io/iiif-img/v3/.../man_10000006.0x000002/info.json"
        );
        assert_eq!(
            infos[0].manifest_label,
            Some("Cotton MS Nero D IV".to_string())
        );
        assert_eq!(infos[0].canvas_label, Some("Front cover".to_string()));
    }

    #[test]
    fn test_legacy_manifest_prefers_service_and_handles_arrays() {
        let json_data = r#"{
          "@type":"sc:Manifest","label":"A Young Lady with a Parrot","sequences":[{"canvases":[
            {"label":"AIC canvas","images":[{"resource":{"@type":"dctypes:Image","@id":"https://www.artic.edu/iiif/2/2d9fb8b5-b9a3-3e41-270e-3c480f7b317b/full/843,/0/default.jpg","service":{"@id":"https://www.artic.edu/iiif/2/2d9fb8b5-b9a3-3e41-270e-3c480f7b317b"}}}]},
            {"label":"Array canvas","images":[{"resource":{"@id":"thumbnail.jpg","service":[{"@id":"svc2","@type":"ImageService2"},{"id":"svc3","type":"ImageService3"}]}}]}
          ]}]
        }"#;

        let manifest: LegacyManifest = serde_json::from_str(json_data).unwrap();
        let infos = manifest.extract_image_infos("https://example.org/manifest.json");

        assert_eq!(infos.len(), 2);
        assert_eq!(
            infos[0].image_uri,
            "https://www.artic.edu/iiif/2/2d9fb8b5-b9a3-3e41-270e-3c480f7b317b/info.json"
        );
        assert_eq!(infos[0].canvas_label, Some("AIC canvas".to_string()));
        assert_eq!(infos[0].canvas_index, 0);
        assert_eq!(infos[1].image_uri, "https://example.org/svc3/info.json");
        assert_eq!(infos[1].canvas_label, Some("Array canvas".to_string()));
        assert_eq!(infos[1].canvas_index, 1);
    }

    #[test]
    fn test_v1_metadata_manifest_uses_legacy_sequence_canvas_images_shape() {
        let json_data = r#"{
          "@context":"http://www.shared-canvas.org/ns/context.json","@type":"sc:Manifest",
          "label":"Book 1","sequences":[{"canvases":[{"label":"p. 1","images":[{"resource":{
            "@id":"http://www.example.org/iiif/book1/res/page1.jpg","@type":"dctypes:Image",
            "service":{"@id":"http://www.example.org/images/book1-page1"}
          }}]}]}]
        }"#;

        let manifest: LegacyManifest = serde_json::from_str(json_data).unwrap();
        let infos = manifest.extract_image_infos("http://www.example.org/iiif/book1/manifest");

        assert_eq!(infos.len(), 1);
        assert_eq!(
            infos[0].image_uri,
            "http://www.example.org/images/book1-page1/info.json"
        );
        assert_eq!(infos[0].manifest_label, Some("Book 1".to_string()));
        assert_eq!(infos[0].canvas_label, Some("p. 1".to_string()));
        assert_eq!(infos[0].canvas_index, 0);
    }

    #[test]
    fn test_empty_annotation_body_or_unsupported() {
        let json_data = r#"
        {
          "id": "manifest-empty-body", "type": "Manifest",
          "items": [{ "id": "c1", "type": "Canvas", "items": [{ "id": "ap1", "type": "AnnotationPage", "items": [
            { "id": "a1", "type": "Annotation", "motivation": "painting", "body": {} },
            { "id": "a2", "type": "Annotation", "motivation": "painting" }
          ]}]}]
        }
        "#;
        let manifest: Manifest = serde_json::from_str(json_data).unwrap();
        let infos = manifest.extract_image_infos("https://example.org/");
        assert_eq!(
            infos.len(),
            0,
            "Expected no image infos from empty or unsupported bodies"
        );
    }

    #[test]
    fn test_uri_resolution_in_extract_image_infos() {
        let manifest_url = "https://example.com/iiif/collection1/bookA/manifest.json";
        let json_data = r#"
        {
          "id": "relative-uri-manifest", "type": "Manifest", "label": {"en": ["Relative Test"]},
          "items": [
            {
              "id": "c1", "type": "Canvas", "label": {"en": ["Canvas 1 Rel Svc"]},
              "items": [{"id": "ap1", "type": "AnnotationPage", "items": [{
                  "id": "a1", "type": "Annotation", "motivation": "painting",
                  "body": { "id": "img1.jpg", "type": "Image", "service": [{"id": "../images/page1_svc", "type": "ImageService2"}]}
              }]}]
            },
            {
              "id": "c2", "type": "Canvas", "label": {"en": ["Canvas 2 Abs Path Svc"]},
              "items": [{"id": "ap2", "type": "AnnotationPage", "items": [{
                  "id": "a2", "type": "Annotation", "motivation": "painting",
                  "body": { "id": "img2.jpg", "type": "Image", "service": [{"id": "/abs/path/to/img_svc", "type": "ImageService3"}]}
              }]}]
            },
            {
              "id": "c3", "type": "Canvas", "label": {"en": ["Canvas 3 Rel Direct Img"]},
              "items": [{"id": "ap3", "type": "AnnotationPage", "items": [{
                  "id": "a3", "type": "Annotation", "motivation": "painting",
                  "body": { "id": "images/cover.jpg", "type": "Image" }
              }]}]
            },
            {
              "id": "c4", "type": "Canvas", "label": {"en": ["Canvas 4 Full URL Svc"]},
              "items": [{"id": "ap4", "type": "AnnotationPage", "items": [{
                  "id": "a4", "type": "Annotation", "motivation": "painting",
                  "body": { "id": "img4.jpg", "type": "Image", "service": [{"id": "https://other.example.net/iiif/itemQ/svc", "type": "ImageService2"}]}
              }]}]
            },
            {
              "id": "c5", "type": "Canvas", "label": {"en": ["Canvas 5 Rel Svc No Slash"]},
              "items": [{"id": "ap5", "type": "AnnotationPage", "items": [{
                  "id": "a5", "type": "Annotation", "motivation": "painting",
                  "body": { "id": "img5.jpg", "type": "Image", "service": [{"id": "images_rel_noslash_svc", "type": "ImageService2"}]}
              }]}]
            }
          ]
        }
        "#;
        let manifest: Manifest = serde_json::from_str(json_data).unwrap();
        let infos = manifest.extract_image_infos(manifest_url);

        assert_eq!(infos.len(), 5);

        assert_eq!(
            infos[0].image_uri,
            "https://example.com/iiif/collection1/images/page1_svc/info.json"
        );
        assert_eq!(infos[0].canvas_label, Some("Canvas 1 Rel Svc".to_string()));

        assert_eq!(
            infos[1].image_uri,
            "https://example.com/abs/path/to/img_svc/info.json"
        );
        assert_eq!(
            infos[1].canvas_label,
            Some("Canvas 2 Abs Path Svc".to_string())
        );

        assert_eq!(
            infos[2].image_uri,
            "https://example.com/iiif/collection1/bookA/images/cover.jpg"
        );
        assert_eq!(
            infos[2].canvas_label,
            Some("Canvas 3 Rel Direct Img".to_string())
        );

        assert_eq!(
            infos[3].image_uri,
            "https://other.example.net/iiif/itemQ/svc/info.json"
        );
        assert_eq!(
            infos[3].canvas_label,
            Some("Canvas 4 Full URL Svc".to_string())
        );

        assert_eq!(
            infos[4].image_uri,
            "https://example.com/iiif/collection1/bookA/images_rel_noslash_svc/info.json"
        );
        assert_eq!(
            infos[4].canvas_label,
            Some("Canvas 5 Rel Svc No Slash".to_string())
        );
    }
}
