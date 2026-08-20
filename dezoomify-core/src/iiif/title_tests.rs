use super::determine_title;
use super::manifest_types::ExtractedImageInfo;

#[test]
fn test_determine_title_all_components() {
    let image_info = ExtractedImageInfo {
        image_uri: "https://example.com/image.json".to_string(),
        manifest_label: Some("Manifest Title".to_string()),
        metadata_title: Some("Metadata Title".to_string()),
        canvas_label: Some("Canvas Label".to_string()),
        canvas_index: 0,
    };

    let result = determine_title(&image_info);
    assert_eq!(
        result,
        Some("Manifest Title - Metadata Title - Canvas Label".to_string())
    );
}

#[test]
fn test_determine_title_manifest_and_canvas_only() {
    let image_info = ExtractedImageInfo {
        image_uri: "https://example.com/image.json".to_string(),
        manifest_label: Some("Book Title".to_string()),
        metadata_title: None,
        canvas_label: Some("Page 1".to_string()),
        canvas_index: 0,
    };

    let result = determine_title(&image_info);
    assert_eq!(result, Some("Book Title - Page 1".to_string()));
}

#[test]
fn test_determine_title_canvas_only() {
    let image_info = ExtractedImageInfo {
        image_uri: "https://example.com/image.json".to_string(),
        manifest_label: None,
        metadata_title: None,
        canvas_label: Some("Single Page".to_string()),
        canvas_index: 0,
    };

    let result = determine_title(&image_info);
    assert_eq!(result, Some("Single Page".to_string()));
}

#[test]
fn test_determine_title_no_duplicates() {
    // Test that duplicate titles are not repeated
    let image_info = ExtractedImageInfo {
        image_uri: "https://example.com/image.json".to_string(),
        manifest_label: Some("Same Title".to_string()),
        metadata_title: Some("Same Title".to_string()), // Duplicate
        canvas_label: Some("Different Label".to_string()),
        canvas_index: 0,
    };

    let result = determine_title(&image_info);
    assert_eq!(result, Some("Same Title - Different Label".to_string()));
}

#[test]
fn test_determine_title_empty() {
    let image_info = ExtractedImageInfo {
        image_uri: "https://example.com/image.json".to_string(),
        manifest_label: None,
        metadata_title: None,
        canvas_label: None,
        canvas_index: 0,
    };

    let result = determine_title(&image_info);
    assert_eq!(result, None);
}

#[test]
fn test_determine_title_metadata_only() {
    let image_info = ExtractedImageInfo {
        image_uri: "https://example.com/image.json".to_string(),
        manifest_label: None,
        metadata_title: Some("Metadata Only".to_string()),
        canvas_label: None,
        canvas_index: 0,
    };

    let result = determine_title(&image_info);
    assert_eq!(result, Some("Metadata Only".to_string()));
}

#[test]
fn test_determine_title_special_characters() {
    let image_info = ExtractedImageInfo {
        image_uri: "https://example.com/image.json".to_string(),
        manifest_label: Some("Ms. Smith's \"Book\" & Notes (1850-1900)".to_string()),
        metadata_title: None,
        canvas_label: Some("Page #1: Introduction/Overview".to_string()),
        canvas_index: 0,
    };

    let result = determine_title(&image_info);
    assert_eq!(
        result,
        Some(
            "Ms. Smith's \"Book\" & Notes (1850-1900) - Page #1: Introduction/Overview".to_string()
        )
    );
}

#[test]
fn test_determine_title_very_long() {
    let long_manifest = "A".repeat(100);
    let long_canvas = "B".repeat(100);

    let image_info = ExtractedImageInfo {
        image_uri: "https://example.com/image.json".to_string(),
        manifest_label: Some(long_manifest.clone()),
        metadata_title: None,
        canvas_label: Some(long_canvas.clone()),
        canvas_index: 0,
    };

    let result = determine_title(&image_info);
    let expected = format!("{long_manifest} - {long_canvas}");
    assert_eq!(result, Some(expected));
}

#[test]
fn test_determine_title_unicode() {
    let image_info = ExtractedImageInfo {
        image_uri: "https://example.com/image.json".to_string(),
        manifest_label: Some("古典文学作品集".to_string()),
        metadata_title: Some("詩經選讀".to_string()),
        canvas_label: Some("第一章：關雎".to_string()),
        canvas_index: 0,
    };

    let result = determine_title(&image_info);
    assert_eq!(
        result,
        Some("古典文学作品集 - 詩經選讀 - 第一章：關雎".to_string())
    );
}

#[test]
fn test_determine_title_whitespace_handling() {
    let image_info = ExtractedImageInfo {
        image_uri: "https://example.com/image.json".to_string(),
        manifest_label: Some("  Manifest with spaces  ".to_string()),
        metadata_title: Some("\tTabbed metadata\t".to_string()),
        canvas_label: Some("Canvas\nwith\nnewlines".to_string()),
        canvas_index: 0,
    };

    let result = determine_title(&image_info);
    // Note: The function doesn't currently trim whitespace, it preserves what's in the manifest
    assert_eq!(
        result,
        Some("  Manifest with spaces   - \tTabbed metadata\t - Canvas\nwith\nnewlines".to_string())
    );
}
