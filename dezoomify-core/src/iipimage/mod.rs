//! Pure discovery for `IIPImage` metadata and tile pyramids.

use std::str::FromStr;
use std::sync::Arc;

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryError, DiscoveryMatch, Grid, ImageCatalog,
    ImageDescriptor, LevelDescriptor, Request, StableId,
};

const META: &str = "&OBJ=Max-size&OBJ=Tile-size&OBJ=Resolution-number";

pub const SPEC: DezoomerSpec = DezoomerSpec::new(
    "iipimage",
    &[
        DiscoveryMatch::UrlPredicate(needs_metadata).map_url(metadata_url),
        DiscoveryMatch::Any.extract(catalog),
    ],
)
.recognizing(is_iip, "not an IIPImage URL")
.preferring(|uri| uri.to_ascii_lowercase().contains("?fif"));

fn is_iip(uri: &str) -> bool {
    uri.ends_with(META) || uri.to_ascii_lowercase().contains("?fif")
}

fn needs_metadata(uri: &str) -> bool {
    !uri.ends_with(META)
}

#[allow(clippy::unnecessary_wraps)]
fn metadata_url(input: &str) -> Result<Request, DiscoveryError> {
    Ok(Request::new(format!(
        "{}{}",
        input
            .chars()
            .take_while(|character| *character != '&')
            .collect::<String>(),
        META
    )))
}

fn catalog(uri: &str, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let metadata = Arc::new(Metadata::try_from(bytes)?);
    let base: Arc<str> = uri.trim_end_matches(META).into();
    let mut levels: Vec<_> = (0..metadata.levels)
        .map(|index| {
            let reverse = metadata.levels - index - 1;
            let size = metadata.size / 2_u32.pow(reverse);
            let base = Arc::clone(&base);
            let source = Grid::with_requests(
                format!("iip:{index}").into(),
                size,
                metadata.tile_size,
                Vec2d::default(),
                move |tile| Request::new(format!("{base}&JTL={index},{}", tile.row_major_ordinal)),
            )
            .map_err(|error| DiscoveryError::Session(format!("invalid IIP grid: {error}")))?;
            Ok(LevelDescriptor::new(source).with_title(Some(format!(
                "IIP level {index} ({: >5}×{: >5} pixels)",
                size.x, size.y,
            ))))
        })
        .collect::<Result<Vec<_>, DiscoveryError>>()?;
    levels.sort_by_key(|level| level.source.image_size().map_or(0, Vec2d::area));
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("iip:image"),
        format: StableId::new("iipimage"),
        levels,
        ..Default::default()
    })]))
}

#[derive(Clone, Debug)]
struct Metadata {
    size: Vec2d,
    tile_size: Vec2d,
    levels: u32,
}
impl FromStr for Metadata {
    type Err = DiscoveryError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let numbers = |name: &str, minimum: usize| {
            value
                .lines()
                .filter_map(|line| {
                    let (key, raw) = line.split_once(':')?;
                    let values: Vec<_> = raw
                        .split_ascii_whitespace()
                        .filter_map(|number| number.parse::<u32>().ok())
                        .collect();
                    (key.trim().eq_ignore_ascii_case(name) && values.len() >= minimum)
                        .then_some(values)
                })
                .next_back()
        };
        let size = numbers("max-size", 2).map(|values| Vec2d {
            x: values[0],
            y: values[1],
        });
        let tile_size = numbers("tile-size", 2).map(|values| Vec2d {
            x: values[0],
            y: values[1],
        });
        let levels = numbers("resolution-number", 1).map(|values| values[0]);
        Ok(Self {
            size: size
                .ok_or_else(|| DiscoveryError::Session("IIP metadata lacks Max-size".into()))?,
            tile_size: tile_size
                .ok_or_else(|| DiscoveryError::Session("IIP metadata lacks Tile-size".into()))?,
            levels: levels.ok_or_else(|| {
                DiscoveryError::Session("IIP metadata lacks Resolution-number".into())
            })?,
        })
    }
}
impl TryFrom<&[u8]> for Metadata {
    type Error = DiscoveryError;
    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        std::str::from_utf8(bytes)
            .map_err(|error| DiscoveryError::Session(error.to_string()))?
            .parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::TileSource;

    #[test]
    fn lowercase_fif_urls_request_canonical_metadata() {
        let uri = "https://publications-images.artic.edu/fcgi-bin/iipsrv.fcgi?fif=osci/Renoir_11/Color_Corrected/G39094sm2.ptif&jtl=4,11";
        assert_eq!(
            metadata_url(uri).unwrap().uri,
            "https://publications-images.artic.edu/fcgi-bin/iipsrv.fcgi?fif=osci/Renoir_11/Color_Corrected/G39094sm2.ptif&OBJ=Max-size&OBJ=Tile-size&OBJ=Resolution-number"
        );
    }

    #[test]
    fn parses_metadata_levels_and_iip_tile_geometry() {
        let metadata = b"Max-size:512 512\nTile-size:256 256\nResolution-number:2";
        let catalog = catalog(
            "http://test.com/&OBJ=Max-size&OBJ=Tile-size&OBJ=Resolution-number",
            metadata,
        )
        .unwrap();
        let CatalogEntry::Ready(image) = catalog.into_entries().pop().unwrap() else {
            panic!("IIP metadata did not produce an image")
        };
        assert_eq!(image.levels.len(), 2);
        assert_eq!(
            image.levels[0].source.image_size(),
            Some(Vec2d { x: 256, y: 256 })
        );
        assert_eq!(
            image.levels[1].source.image_size(),
            Some(Vec2d { x: 512, y: 512 })
        );
        let TileSource::Grid(low_plan) = &image.levels[0].source else {
            panic!("IIP levels must be grids")
        };
        assert_eq!(low_plan.count(), 1);
        assert_eq!(
            low_plan
                .tiles_row_major()
                .next()
                .unwrap()
                .unwrap()
                .request
                .uri,
            "http://test.com/&JTL=0,0"
        );
        let TileSource::Grid(plan) = &image.levels[1].source else {
            panic!("IIP levels must be grids")
        };
        assert_eq!(plan.count(), 4);
        let tiles: Vec<_> = plan.tiles_row_major().map(Result::unwrap).collect();
        assert_eq!(tiles[0].request.uri, "http://test.com/&JTL=1,0");
        assert_eq!(tiles[2].request.uri, "http://test.com/&JTL=1,2");
    }

    #[test]
    fn metadata_parsing_reports_missing_fields_and_invalid_utf8() {
        let error = "Max-size:512 512"
            .parse::<Metadata>()
            .unwrap_err()
            .to_string();
        assert!(error.contains("Tile-size"));
        let error = Metadata::try_from(&[0xff][..]).unwrap_err().to_string();
        assert!(error.contains("UTF") || error.contains("utf"));
        let parsed: Metadata = "Max-size:23235 23968\nTile-size:256 256\nResolution-number:9"
            .parse()
            .unwrap();
        assert_eq!(parsed.size, Vec2d { x: 23235, y: 23968 });
        assert_eq!(parsed.tile_size, Vec2d { x: 256, y: 256 });
        assert_eq!(parsed.levels, 9);
    }
}
