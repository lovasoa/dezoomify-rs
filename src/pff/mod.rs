/// Dezoomer for the zoomify PFF servlet API format
/// See: https://github.com/lovasoa/pff-extract/wiki/Zoomify-PFF-file-format-documentation

use serde_urlencoded;

use custom_error::custom_error;
use image_properties::PffHeader;

use crate::dezoomer::*;

mod image_properties;

#[derive(Default)]
pub struct PFF;

custom_error! {pub PffError
    EncodeError{source: serde_urlencoded::de::Error} = "Invalid meta information file: {source}"
}

impl From<PffError> for DezoomerError {
    fn from(err: PffError) -> Self {
        DezoomerError::Other { source: err.into() }
    }
}

impl Dezoomer for PFF {
    fn name(&self) -> &'static str {
        "pff"
    }

    fn zoom_levels(&mut self, data: &DezoomerInput) -> Result<ZoomLevels, DezoomerError> {
        self.assert(data.uri.contains(".pff&requestType=1"))?;
        let with_contents = data.with_contents()?;
        let contents = with_contents.contents;
        let uri = with_contents.uri;
        Ok(zoom_levels(uri, contents)?)
    }
}

fn zoom_levels(url: &str, raw_info: &[u8]) -> Result<ZoomLevels, PffError> {
    todo!()
}

struct PffZoomLevel {
    tile_size: Vec2d
}

impl TilesRect for PffZoomLevel {
    fn size(&self) -> Vec2d {
        todo!()
    }

    fn tile_size(&self) -> Vec2d {
        self.tile_size
    }

    fn tile_url(&self, col_and_row_pos: Vec2d) -> String { todo!() }
}

impl std::fmt::Debug for PffZoomLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        todo!()
    }
}
