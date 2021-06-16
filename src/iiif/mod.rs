use std::sync::Arc;

use custom_error::custom_error;
use log::{debug, info};

use tile_info::ImageInfo;

use crate::dezoomer::*;
use crate::iiif::tile_info::TileSizeFormat;
use crate::json_utils::all_json;
use crate::max_size_in_rect;

pub mod tile_info;

/// Dezoomer for the International Image Interoperability Framework.
/// See https://iiif.io/
#[derive(Default)]
pub struct IIIF;

custom_error! {pub IIIFError
    JsonError{source: serde_json::Error} = "Invalid IIIF info.json file: {source}"
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
                        debug!("keeping image info {:?} because it has distinctive IIIF properties", info)
                    } else {
                        info!("dropping level {:?}", info)
                    }
                    keep
                })
                .flat_map(|info| zoom_levels_from_info(url, info))
                .collect();
            if levels.is_empty() {
                Err(e.into())
            } else {
                info!("No normal info.json parsing failed ({}), \
                but {} inline json5 zoom level(s) were found.",
                      e, levels.len());
                Ok(levels)
            }
        }
    }
}

fn zoom_levels_from_info(url: &str, mut image_info: ImageInfo) -> ZoomLevels {
    image_info.remove_test_id();
    let img = Arc::new(image_info);
    let tiles = img.tiles();
    let base_url = &Arc::from(url.replace("/info.json", ""));
    let levels = tiles
        .iter()
        .flat_map(|tile_info| {
            let tile_size = tile_info.size();
            let quality = Arc::from(img.best_quality());
            let format = Arc::from(img.best_format());
            let size_format = img.preferred_size_format();
            info!("Chose the following image parameters: tile_size=({}) quality={} format={}",
                  tile_size, quality, format);
            let page_info = &img; // Required to allow the move
            tile_info
                .scale_factors
                .iter()
                .map(move |&scale_factor| IIIFZoomLevel {
                    scale_factor,
                    tile_size,
                    page_info: Arc::clone(page_info),
                    base_url: Arc::clone(base_url),
                    quality: Arc::clone(&quality),
                    format: Arc::clone(&format),
                    size_format,
                })
        })
        .into_zoom_levels();
    levels
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
            base = self.page_info.id.as_deref().unwrap_or_else(|| self.base_url.as_ref()),
            x = xy_pos.x,
            y = xy_pos.y,
            img_w = scaled_tile_size.x,
            img_h = scaled_tile_size.y,
            tile_size = TileSizeFormatter { w: tile_size.x, h: tile_size.y, format: self.size_format },
            rotation = 0,
            quality = self.quality,
            format = self.format,
        )
    }
}

struct TileSizeFormatter { w: u32, h: u32, format: TileSizeFormat }

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
            .base_url
            .split('/')
            .last()
            .and_then(|s: &str| {
                let s = s.trim();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            })
            .unwrap_or("IIIF Image");
        write!(f, "{}", name)
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
    assert_eq!(tiles, vec![
        "http://www.asmilano.it/fast/iipsrv.fcgi?IIIF=/opt/divenire/files/./tifs/05/36/536765.tif/0,0,15001,32768/234,512/0/default.jpg",
        "http://www.asmilano.it/fast/iipsrv.fcgi?IIIF=/opt/divenire/files/./tifs/05/36/536765.tif/0,32768,15001,15234/234,238/0/default.jpg",
    ])
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
    assert_eq!(tiles, vec![
        "http://ophir.dev/0,0,512,512/512,512/0/default.jpg",
        "http://ophir.dev/512,0,512,512/512,512/0/default.jpg",
        "http://ophir.dev/0,512,512,512/512,512/0/default.jpg",
        "http://ophir.dev/512,512,512,512/512,512/0/default.jpg",
    ])
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
    assert!(res.is_err(), "openseadragon zoomify image should not be misdetected");
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
    assert_eq!(level.size_hint(), Some(Vec2d { x: 515, y: 381 }));
    let tiles: Vec<String> = level
        .next_tiles(None)
        .into_iter()
        .map(|t| t.url)
        .collect();
    assert_eq!(tiles, vec![
        "https://images.britishart.yale.edu/iiif/fd470c3e-ead0-4878-ac97-d63295753f82/0,0,5156,3816/515,381/0/native.png",
    ])
}
