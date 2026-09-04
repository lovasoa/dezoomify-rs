//! Pure discovery for `XLimage` `*.img?cmd=info` documents.

use std::sync::Arc;

use serde::Deserialize;

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryError, DiscoveryMatch, DiscoveryRoute, Grid, ImageCatalog,
    ImageDescriptor, LevelDescriptor, Request, StableId,
};

const INFO_QUERY: &str = "cmd=info";

const ROUTES: &[DiscoveryRoute] = &[DiscoveryMatch::Any.extract(catalog)];

pub const SPEC: DezoomerSpec = DezoomerSpec::new("xlimage", ROUTES)
    .recognizing(is_xlimage_url, "not an XLimage URL")
    .preferring(is_info_url);

fn is_xlimage_url(uri: &str) -> bool {
    let path = uri.split_once(['?', '#']).map_or(uri, |(path, _)| path);
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit('.').next().is_some_and(|extension| {
        let extension = extension.to_ascii_lowercase();
        extension == "imgf" || extension == "imgi"
    })
}

fn is_info_url(uri: &str) -> bool {
    uri.to_ascii_lowercase().contains(INFO_QUERY)
}

fn image_origin(url: &str) -> String {
    url.split_once(['?', '#'])
        .map_or(url, |(path, _)| path)
        .to_owned()
}

fn catalog(url: &str, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let metadata: Metadata = serde_xml_rs::from_reader(bytes).map_err(|error| {
        DiscoveryError::Session(format!("unable to parse XLimage metadata: {error}"))
    })?;
    if metadata.width == 0
        || metadata.height == 0
        || metadata.tileside == 0
        || metadata.maxzoom == 0
    {
        return Err(DiscoveryError::Session(
            "XLimage metadata must declare positive width, height, tileside, and maxzoom".into(),
        ));
    }
    let origin: Arc<str> = image_origin(url).into();
    let levels = build_levels(&metadata, &origin)?;
    let title = image_title(&origin);

    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("xlimage:image"),
        title,
        format: StableId::new("xlimage"),
        levels,
        ..Default::default()
    })]))
}

fn image_title(origin: &str) -> Option<String> {
    origin
        .split('?')
        .filter(|part| part.to_ascii_lowercase().contains(".img"))
        .filter_map(|part| part.rsplit('/').next())
        .find_map(|file| {
            let stem = file.split('.').next()?;
            (!stem.is_empty()).then(|| stem.to_owned())
        })
}

fn build_levels(
    metadata: &Metadata,
    origin: &Arc<str>,
) -> Result<Vec<LevelDescriptor>, DiscoveryError> {
    let mut levels = Vec::new();
    let mut zoom = 1;
    loop {
        let width = metadata.width.div_ceil(zoom);
        let height = metadata.height.div_ceil(zoom);
        let origin = Arc::clone(origin);
        let source = Grid::with_requests(
            StableId::new(format!("xlimage:{zoom}")),
            Vec2d {
                x: width,
                y: height,
            },
            Vec2d::square(metadata.tileside),
            Vec2d::default(),
            move |tile| {
                let coord: Vec2d = tile.coord.into();
                Request::new(format!(
                    "{origin}?cmd=tile&x={}&y={}&z={zoom}",
                    coord.x, coord.y
                ))
            },
        )
        .map_err(|error| DiscoveryError::Session(format!("invalid XLimage grid: {error}")))?;
        levels.push(
            LevelDescriptor::new(source)
                .with_scale_factor(Some(zoom))
                .with_title(Some(format!("XLimage level {zoom}"))),
        );

        if zoom >= metadata.maxzoom {
            break;
        }
        zoom = zoom
            .checked_mul(2)
            .map_or(metadata.maxzoom, |next| next.min(metadata.maxzoom));
    }
    Ok(levels)
}

#[derive(Debug, Deserialize)]
struct Metadata {
    width: u32,
    height: u32,
    tileside: u32,
    #[serde(default = "default_maxzoom")]
    maxzoom: u32,
}

fn default_maxzoom() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_title_is_the_img_file_stem() {
        assert_eq!(
            image_title("https://uffizicloud.centrica.it/7711/closer/hi-res/A1456.imgf"),
            Some("A1456".to_owned())
        );
        assert_eq!(image_title("https://fixtures.test/xl/viewer.php"), None);
    }
}
