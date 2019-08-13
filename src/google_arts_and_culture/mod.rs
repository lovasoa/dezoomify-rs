use std::error::Error;
use std::sync::Arc;

use tile_info::{PageInfo, TileInfo};

use crate::dezoomer::*;

mod decryption;
mod tile_info;
mod url;

#[derive(Default)]
pub struct GAPDezoomer {
    page_info: Option<Arc<PageInfo>>,
}

impl Dezoomer for GAPDezoomer {
    fn name(&self) -> &'static str {
        "google_arts_and_culture"
    }

    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        self.assert(data.uri.contains("artsandculture.google.com") || self.page_info.is_some())?;
        let contents = data.with_contents()?.contents;
        match &self.page_info {
            None => {
                let page_source = std::str::from_utf8(contents).map_err(DezoomerError::wrap)?;
                let info: PageInfo = page_source.parse().map_err(DezoomerError::wrap)?;
                let uri = info.tile_info_url();
                self.page_info = Some(Arc::new(info));
                Err(DezoomerError::NeedsData { uri })
            }
            Some(page_info) => {
                let TileInfo {
                    tile_width,
                    tile_height,
                    pyramid_level,
                    ..
                } = serde_xml_rs::from_reader(contents).map_err(DezoomerError::wrap)?;
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
                Ok(levels)
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

    fn post_process_tile(
        &self,
        _tile: &TileReference,
        data: Vec<u8>,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        Ok(decryption::decrypt(data)?)
    }
}

impl std::fmt::Debug for GAPZoomLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", &self.page_info.name)
    }
}
