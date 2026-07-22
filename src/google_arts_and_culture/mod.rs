use std::error::Error;
use std::sync::Arc;

use tile_info::{PageInfo, TileInfo};

use crate::dezoomer::{
    Dezoomer, DezoomerError, DezoomerInput, Images, IntoZoomLevels, PostProcessFn, TileReference,
    TilesRect, Vec2d, ZoomLevels,
};

mod decryption;
mod tile_info;
mod url;

/// A dezoomer for google arts and culture.
/// It takes an url to an artwork page as input.
#[derive(Default)]
pub struct GAPDezoomer {
    page_info: Option<Arc<PageInfo>>,
}

impl Dezoomer for GAPDezoomer {
    fn name(&self) -> &'static str {
        "google_arts_and_culture"
    }

    fn images(&mut self, data: &DezoomerInput) -> Result<Images, DezoomerError> {
        // Allow Google Arts & Culture URLs or tile info URLs when we have page_info
        let is_valid_uri = data.uri.contains("artsandculture.google.com")
            || (self.page_info.is_some() && data.uri.ends_with("=g"));
        self.assert(is_valid_uri)?;

        let contents = data.with_contents()?.contents;
        match &self.page_info {
            None => {
                let page_source = std::str::from_utf8(contents).map_err(DezoomerError::wrap)?;
                let info: PageInfo = page_source.parse().map_err(DezoomerError::wrap)?;
                log::debug!("Decoded google arts page info: {info:?}");
                let uri = info.tile_info_url();
                self.page_info = Some(Arc::new(info));
                Err(DezoomerError::NeedsData { uri })
            }
            Some(page_info) => {
                log::debug!("Attempting to parse tile info XML from {}", data.uri);

                // Debug: Log the first few bytes of the response to see what we're getting
                let response_preview = if contents.len() > 100 {
                    String::from_utf8_lossy(&contents[..100])
                } else {
                    String::from_utf8_lossy(contents)
                };
                log::debug!("Tile info response preview: {response_preview}");

                let TileInfo {
                    tile_width,
                    tile_height,
                    pyramid_level,
                    ..
                } = serde_xml_rs::from_reader(contents).map_err(|e| {
                    log::error!(
                        "Failed to parse tile info XML: {}. Response was: {}",
                        e,
                        String::from_utf8_lossy(contents)
                    );
                    DezoomerError::wrap(e)
                })?;

                log::debug!(
                    "Successfully parsed tile info: {}x{} tiles, {} levels",
                    tile_width,
                    tile_height,
                    pyramid_level.len()
                );

                let levels: ZoomLevels = pyramid_level
                    .into_iter()
                    .enumerate()
                    .map(|(z, level)| {
                        let width = tile_width * level.num_tiles_x - level.empty_pels_x;
                        let height = tile_height * level.num_tiles_y - level.empty_pels_y;
                        GAPZoomLevel {
                            size: Vec2d {
                                x: width,
                                y: height,
                            },
                            tile_size: Vec2d {
                                x: tile_width,
                                y: tile_height,
                            },
                            z,
                            page_info: Arc::clone(page_info),
                        }
                    })
                    .into_zoom_levels();
                Ok(levels.into())
            }
        }
    }
}

struct GAPZoomLevel {
    size: Vec2d,
    tile_size: Vec2d,
    z: usize,
    page_info: Arc<PageInfo>,
}

impl TilesRect for GAPZoomLevel {
    fn size(&self) -> Vec2d {
        self.size
    }

    fn tile_size(&self) -> Vec2d {
        self.tile_size
    }

    fn tile_url(&self, pos: Vec2d) -> String {
        let Vec2d { x, y } = pos;
        url::compute_url(&self.page_info, x, y, self.z)
    }

    fn post_process_fn(&self) -> PostProcessFn {
        PostProcessFn::Fn(post_process_tile)
    }

    fn title(&self) -> Option<String> {
        Some(format!("{self:?}"))
    }
}

fn post_process_tile(
    _tile: &TileReference,
    data: Vec<u8>,
) -> Result<Vec<u8>, Box<dyn Error + Send + 'static>> {
    decryption::decrypt(data).map_err(|e| Box::new(e) as Box<dyn Error + Send + 'static>)
}

impl std::fmt::Debug for GAPZoomLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.page_info.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dezoomer::test_utils::{expect_needs_data, expect_single_resolved};
    use crate::dezoomer::{DezoomerInput, PageContents};
    use std::fs;
    use std::path::Path;

    fn get_test_page_html() -> Vec<u8> {
        let test_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("google_arts_and_culture")
            .join("page_source.html");
        fs::read(test_path).expect("Failed to read test page source")
    }

    fn get_test_tile_info_xml() -> Vec<u8> {
        let test_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("google_arts_and_culture")
            .join("tile_info.xml");
        fs::read(test_path).expect("Failed to read test tile info XML")
    }

    #[test]
    fn test_parse_google_arts_page() {
        let mut dezoomer = GAPDezoomer::default();
        let page_html = get_test_page_html();

        let input = DezoomerInput {
            uri: "https://artsandculture.google.com/asset/test".to_string(),
            contents: PageContents::Success(page_html),
        };

        // First call should extract page info and request tile info URL
        let uri = expect_needs_data(dezoomer.images(&input));
        assert!(uri.ends_with("=g"));
        assert!(uri.contains("lh5.ggpht.com"));

        // Dezoomer should now have page_info stored
        assert!(dezoomer.page_info.is_some());
    }

    #[test]
    fn test_parse_tile_info() {
        let mut dezoomer = GAPDezoomer::default();

        // First, set up page_info manually (normally this would be set by the first call)
        let page_info = PageInfo {
            base_url: "https://lh5.ggpht.com/test".to_string(),
            token: "test_token".to_string(),
            name: "Test Image".to_string(),
        };
        dezoomer.page_info = Some(Arc::new(page_info));

        let tile_info_xml = get_test_tile_info_xml();

        let input = DezoomerInput {
            uri: "https://lh5.ggpht.com/test=g".to_string(),
            contents: PageContents::Success(tile_info_xml),
        };

        // Second call should parse tile info and return zoom levels
        let levels = expect_single_resolved(dezoomer.images(&input).unwrap()).into_zoom_levels();
        assert_eq!(levels.len(), 5); // Based on our test XML

        // Verify the largest level
        let largest_level = &levels[4];
        assert_eq!(largest_level.size_hint(), Some(Vec2d { x: 5436, y: 4080 })); // 11*512-196, 8*512-16
    }

    #[test]
    fn test_full_workflow() {
        let mut dezoomer = GAPDezoomer::default();

        // Step 1: Parse Google Arts page
        let page_html = get_test_page_html();
        let input1 = DezoomerInput {
            uri: "https://artsandculture.google.com/asset/test".to_string(),
            contents: PageContents::Success(page_html),
        };

        let tile_info_uri = expect_needs_data(dezoomer.images(&input1));

        // Step 2: Parse tile info
        let tile_info_xml = get_test_tile_info_xml();
        let input2 = DezoomerInput {
            uri: tile_info_uri,
            contents: PageContents::Success(tile_info_xml),
        };

        let levels = expect_single_resolved(dezoomer.images(&input2).unwrap()).into_zoom_levels();
        assert_eq!(levels.len(), 5);

        // Test that levels have the expected properties
        let level = &levels[0];
        assert!(level.size_hint().is_some());
        let name = level.name();
        assert!(name.contains("©Designers Anonymes")); // Name extracted from test data
    }

    #[test]
    fn test_url_validation() {
        let mut dezoomer = GAPDezoomer::default();

        // Should accept Google Arts & Culture URLs
        let valid_input = DezoomerInput {
            uri: "https://artsandculture.google.com/asset/test".to_string(),
            contents: PageContents::Success(vec![]),
        };
        // This will fail because contents are empty, but URL validation should pass
        let result = dezoomer.images(&valid_input);
        assert!(matches!(
            result,
            Err(DezoomerError::DownloadError { .. } | DezoomerError::Other { .. })
        ));

        // Should reject non-Google Arts URLs when no page_info is set
        let invalid_input = DezoomerInput {
            uri: "https://example.com/test".to_string(),
            contents: PageContents::Success(vec![]),
        };
        let result = dezoomer.images(&invalid_input);
        assert!(matches!(result, Err(DezoomerError::WrongDezoomer { .. })));

        // Should accept tile info URLs when page_info is set
        dezoomer.page_info = Some(Arc::new(PageInfo {
            base_url: "https://lh5.ggpht.com/test".to_string(),
            token: "test_token".to_string(),
            name: "Test Image".to_string(),
        }));

        let tile_info_input = DezoomerInput {
            uri: "https://lh5.ggpht.com/test=g".to_string(),
            contents: PageContents::Success(vec![]),
        };
        // This will fail because contents are empty, but URL validation should pass
        let result = dezoomer.images(&tile_info_input);
        assert!(!matches!(result, Err(DezoomerError::WrongDezoomer { .. })));
    }

    #[test]
    fn test_invalid_tile_info_xml() {
        let mut dezoomer = GAPDezoomer {
            page_info: Some(Arc::new(PageInfo {
                base_url: "https://lh5.ggpht.com/test".to_string(),
                token: "test_token".to_string(),
                name: "Test Image".to_string(),
            })),
        };

        let invalid_xml = b"<invalid>not a tile info</invalid>";
        let input = DezoomerInput {
            uri: "https://lh5.ggpht.com/test=g".to_string(),
            contents: PageContents::Success(invalid_xml.to_vec()),
        };

        assert!(matches!(
            dezoomer.images(&input),
            Err(DezoomerError::Other { .. })
        ));
    }

    #[test]
    fn test_tile_url_generation() {
        let page_info = Arc::new(PageInfo {
            base_url: "https://lh5.ggpht.com/test".to_string(),
            token: "test_token".to_string(),
            name: "Test Image".to_string(),
        });

        let level = GAPZoomLevel {
            size: Vec2d { x: 1024, y: 768 },
            tile_size: Vec2d { x: 256, y: 256 },
            z: 2,
            page_info: Arc::clone(&page_info),
        };

        let tile_url = level.tile_url(Vec2d { x: 1, y: 1 });
        assert!(tile_url.starts_with("https://lh5.ggpht.com/test"));
        assert!(tile_url.contains("=x1-y1-z2-t"));

        // URL should contain the HMAC signature
        assert!(tile_url.len() > page_info.base_url.len() + 20);
    }

    #[test]
    fn test_dezoomer_name() {
        let dezoomer = GAPDezoomer::default();
        assert_eq!(dezoomer.name(), "google_arts_and_culture");
    }

    #[test]
    fn test_zoom_level_debug() {
        let page_info = Arc::new(PageInfo {
            base_url: "https://lh5.ggpht.com/test".to_string(),
            token: "test_token".to_string(),
            name: "Test Image Name".to_string(),
        });

        let level = GAPZoomLevel {
            size: Vec2d { x: 1024, y: 768 },
            tile_size: Vec2d { x: 256, y: 256 },
            z: 2,
            page_info: Arc::clone(&page_info),
        };

        let debug_str = format!("{level:?}");
        assert_eq!(debug_str, "Test Image Name");
    }
}
