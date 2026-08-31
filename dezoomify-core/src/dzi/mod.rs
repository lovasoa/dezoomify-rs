//! Pure discovery for Deep Zoom Image descriptors.

use std::sync::{Arc, LazyLock};

use dzi_file::DziFile;
use regex::{Regex, bytes::Regex as BytesRegex};

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryContext, DiscoveryError, DiscoveryMatch,
    DiscoveryResource, DiscoveryRoute, DiscoveryStep, Grid, ImageCatalog, ImageDescriptor,
    LevelDescriptor, Request, StableId, resolve_relative,
};
use crate::json_utils::all_json;

mod dzi_file;

static TILE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("_files/\\d+/\\d+_\\d+\\.(jpe?g|png)$").expect("constant DZI tile pattern")
});
static SEADRAGON_EMBED: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(
        r#"(?is)\bseadragon\s*\.\s*embed\s*\([^,]*,[^,]*,\s*["'](?P<metadata>[^"']+)["']"#,
    )
    .expect("constant Seadragon embed pattern")
});
static PRADO_PYRAMID: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(
        r#"(?is)\bdata-pyr\s*=\s*["'](?P<origin>[^"']+)["'][^>]*\bdata-width\s*=\s*["'](?P<width>\d+)["'][^>]*\bdata-height\s*=\s*["'](?P<height>\d+)["']"#,
    )
    .expect("constant Prado pyramid pattern")
});
static DEEPZOOM_MANIFEST: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(r#"(?is)\bdeepZoomManifest\b["']?\s*[:=]\s*["'](?P<metadata>[^"']+\.dzi)["']"#)
        .expect("constant Deep Zoom manifest pattern")
});

const ROUTES: &[DiscoveryRoute] = &[
    DiscoveryMatch::UrlPredicate(is_tile_url).map_url(tile_metadata),
    DiscoveryMatch::UrlPredicate(is_paris_ark).map_url(paris_reader),
    DiscoveryMatch::ContentPredicate(contains_prado_pyramid).then(load_prado_pyramid),
    DiscoveryMatch::ContentPredicate(contains_deepzoom_manifest).then(follow_deepzoom_manifest),
    DiscoveryMatch::ContentPredicate(contains_seadragon_embed).then(follow_seadragon_embed),
    DiscoveryMatch::Any.extract(load_catalog),
];

pub const SPEC: DezoomerSpec = DezoomerSpec::new("deepzoom", ROUTES).preferring(|uri| {
    uri.contains(".dzi")
        || uri.contains("_files/")
        || uri.contains("bibliotheques-specialisees.paris.fr/ark:/")
});

fn is_tile_url(input: &str) -> bool {
    TILE_URL.is_match(input)
}

fn tile_metadata(input: &str) -> Result<Request, DiscoveryError> {
    let matched = TILE_URL
        .find(input)
        .ok_or_else(|| DiscoveryError::Session("not a DZI tile URL".into()))?;
    Ok(Request::new(format!("{}.dzi", &input[..matched.start()])))
}

fn is_paris_ark(uri: &str) -> bool {
    uri.starts_with("https://bibliotheques-specialisees.paris.fr/ark:/")
}

fn paris_reader(uri: &str) -> Result<Request, DiscoveryError> {
    let ark = uri
        .strip_prefix("https://bibliotheques-specialisees.paris.fr/ark:")
        .filter(|ark| ark.split('/').filter(|part| !part.is_empty()).count() >= 3)
        .ok_or_else(|| DiscoveryError::Session("invalid Paris ARK URL".into()))?;
    let mut parts = ark.split('/').filter(|part| !part.is_empty());
    let prefix = format!(
        "/{}/{}/{}",
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default()
    );
    Ok(Request::new(format!(
        "https://bibliotheques-specialisees.paris.fr/in/imageReader.xhtml?id=ark:{prefix}&updateUrl=updateUrl1653&ark={ark}&selectedTab=otherdocs"
    )))
}

fn contains_seadragon_embed(contents: &[u8]) -> bool {
    SEADRAGON_EMBED.is_match(contents)
}

fn follow_seadragon_embed(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let metadata = SEADRAGON_EMBED
        .captures(resource.bytes())
        .and_then(|captures| captures.name("metadata"))
        .map(|capture| std::str::from_utf8(capture.as_bytes()))
        .transpose()
        .map_err(|_| DiscoveryError::Session("Seadragon metadata URL is not UTF-8".into()))?
        .ok_or_else(|| DiscoveryError::Session("Seadragon embed lacks a metadata URL".into()))?;
    Ok(DiscoveryStep::Follow(Request::new(resolve_relative(
        resource.final_uri(),
        metadata,
    ))))
}

fn contains_prado_pyramid(contents: &[u8]) -> bool {
    PRADO_PYRAMID.is_match(contents)
}

fn contains_deepzoom_manifest(contents: &[u8]) -> bool {
    DEEPZOOM_MANIFEST.is_match(contents)
}

fn follow_deepzoom_manifest(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let metadata = DEEPZOOM_MANIFEST
        .captures(resource.bytes())
        .and_then(|captures| captures.name("metadata"))
        .map(|capture| std::str::from_utf8(capture.as_bytes()))
        .transpose()
        .map_err(|_| DiscoveryError::Session("Deep Zoom manifest URL is not UTF-8".into()))?
        .ok_or_else(|| DiscoveryError::Session("page lacks Deep Zoom manifest URL".into()))?;
    Ok(DiscoveryStep::Follow(Request::new(resolve_relative(
        resource.final_uri(),
        metadata,
    ))))
}

fn load_prado_pyramid(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let captures = PRADO_PYRAMID
        .captures(resource.bytes())
        .ok_or_else(|| DiscoveryError::Session("Prado page lacks pyramid metadata".into()))?;
    let capture = |name| {
        captures
            .name(name)
            .ok_or_else(|| DiscoveryError::Session(format!("Prado page lacks {name}")))
    };
    let origin = std::str::from_utf8(capture("origin")?.as_bytes())
        .map_err(|_| DiscoveryError::Session("Prado tile origin is not UTF-8".into()))?;
    let width = std::str::from_utf8(capture("width")?.as_bytes())
        .ok()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| DiscoveryError::Session("invalid Prado image width".into()))?;
    let height = std::str::from_utf8(capture("height")?.as_bytes())
        .ok()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| DiscoveryError::Session("invalid Prado image height".into()))?;
    let image = DziFile {
        overlap: 1,
        tile_size: 256,
        format: "jpg".into(),
        size: dzi_file::Size { width, height },
        base_url: Some(resolve_relative(resource.final_uri(), origin)),
    };
    Ok(DiscoveryStep::Complete(catalog_from_dzi(
        resource.final_uri(),
        [image],
    )?))
}

fn load_catalog(url: &str, contents: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let xml_result = serde_xml_rs::from_reader::<'_, DziFile, _>(contents);
    let xml_err = xml_result.as_ref().err().map(ToString::to_string);
    let parsed = xml_result
        .ok()
        .into_iter()
        .chain(all_json::<DziFile>(contents))
        .collect::<Vec<_>>();
    if parsed.is_empty() {
        let detail = xml_err.map(|e| format!(": {e}")).unwrap_or_default();
        return Err(DiscoveryError::Session(format!(
            "unable to parse DZI metadata{detail}"
        )));
    }
    catalog_from_dzi(url, parsed)
}

fn catalog_from_dzi(
    url: &str,
    images: impl IntoIterator<Item = DziFile>,
) -> Result<ImageCatalog, DiscoveryError> {
    let mut entries = Vec::new();
    for (image_index, image) in images.into_iter().enumerate() {
        if image.tile_size == 0 {
            return Err(DiscoveryError::Session("invalid DZI zero tile size".into()));
        }
        if image.get_size().x == 0 || image.get_size().y == 0 {
            return Err(DiscoveryError::Session(
                "invalid DZI zero image size".into(),
            ));
        }
        let base_url: Arc<str> = image.base_url(url).into();
        let image_size = image.get_size();
        let tile_size = image.get_tile_size();
        let max_level = image.max_level();
        let mut levels: Vec<_> = std::iter::successors(Some(image_size), |size| {
            (size.x > 1 || size.y > 1).then(|| size.ceil_div(Vec2d::square(2)))
        })
        .zip((0..=max_level).rev())
        .enumerate()
        .map(|(ordinal, (size, zoom))| {
            let base_url = Arc::clone(&base_url);
            let format = image.format.clone();
            let source = Grid::with_requests(
                format!("dzi:{image_index}:{ordinal}").into(),
                size,
                tile_size,
                Vec2d::square(image.overlap),
                move |tile| {
                    let cell: Vec2d = tile.coord.into();
                    Request::new(format!("{base_url}/{zoom}/{}_{}.{format}", cell.x, cell.y))
                },
            )
            .map_err(|error| DiscoveryError::Session(format!("invalid DZI grid: {error}")))?;
            Ok(LevelDescriptor::new(source).with_title(Some(format!(
                "DZI level {ordinal} ({: >5}×{: >5} pixels)",
                size.x, size.y,
            ))))
        })
        .collect::<Result<Vec<_>, DiscoveryError>>()?;
        levels.reverse();
        let title = base_url
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .map(|s| s.trim_end_matches("_files").to_owned());
        entries.push(CatalogEntry::Ready(ImageDescriptor {
            id: StableId::new(format!("dzi:{image_index}")),
            title,
            format: StableId::new("deepzoom"),
            levels,
            ..Default::default()
        }));
    }
    Ok(ImageCatalog::new(entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::TileSource;

    fn ready_image(catalog: ImageCatalog) -> ImageDescriptor {
        match catalog.into_entries().pop().unwrap() {
            CatalogEntry::Ready(image) => image,
            CatalogEntry::Deferred(_) => panic!("DZI is resolved"),
        }
    }

    #[test]
    fn panorama_preserves_urls_overlap_and_normalized_level_order() {
        let contents = br#"<Image TileSize="256" Overlap="2" Format="jpg"><Size Width="600" Height="300"/></Image>"#;
        let catalog = load_catalog("http://x.fr/y/test.dzi", contents).unwrap();
        let CatalogEntry::Ready(image) = &catalog.entries()[0] else {
            panic!("DZI is immediately ready")
        };
        assert_eq!(image.title.as_deref(), Some("test"));
        let levels = ready_image(catalog).levels;
        assert_eq!(levels.len(), 11);
        assert!(
            levels
                .windows(2)
                .all(|pair| pair[0].source.image_size().unwrap().area()
                    <= pair[1].source.image_size().unwrap().area())
        );
        let TileSource::Grid(plan) = &levels[9].source else {
            panic!("DZI is a grid");
        };
        let urls: Vec<_> = plan
            .tiles_row_major()
            .take(10)
            .map(Result::unwrap)
            .map(|tile| tile.request.uri)
            .collect();
        assert_eq!(
            urls,
            vec![
                "http://x.fr/y/test_files/9/0_0.jpg",
                "http://x.fr/y/test_files/9/1_0.jpg"
            ]
        );
    }

    #[test]
    fn parses_xml_with_bom_and_openseadragon_configuration() {
        let bom = "\u{feff}<Image TileSize=\"256\" Overlap=\"0\" Format=\"jpg\"><Size Width=\"6261\" Height=\"6047\"/></Image>";
        let catalog = load_catalog("http://test.com/test.xml", bom.as_bytes()).unwrap();
        assert_eq!(catalog.len(), 1);
        let image = ready_image(catalog);
        assert_eq!(
            image.levels.last().unwrap().source.image_size(),
            Some(Vec2d { x: 6261, y: 6047 })
        );
        let script = r#"OpenSeadragon({tileSources:{Image:{Url:"/example-images/highsmith/highsmith_files/",Format:"jpg",Overlap:"2",TileSize:"256",Size:{Height:"9221",Width:"7026"}}}});"#;
        let levels =
            ready_image(load_catalog("http://test.com/x/test.xml", script.as_bytes()).unwrap())
                .levels;
        let large = levels.last().unwrap();
        assert_eq!(large.source.image_size(), Some(Vec2d { x: 7026, y: 9221 }));
        let TileSource::Grid(plan) = &large.source else {
            unreachable!()
        };
        assert_eq!(
            plan.tiles_row_major().next().unwrap().unwrap().request.uri,
            "http://test.com/example-images/highsmith/highsmith_files/14/0_0.jpg"
        );
    }
}
