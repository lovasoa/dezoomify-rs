use std::sync::LazyLock;

use regex::bytes::Regex as BytesRegex;

use crate::core::{
    DiscoveryContext, DiscoveryError, DiscoveryResource, DiscoveryStep, resolve_relative,
};

use super::catalog_from_dzi;
use super::dzi_file::{DziFile, Size};

static PYRAMID: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(
        r#"(?is)\bdata-pyr\s*=\s*["'](?P<origin>[^"']+)["'][^>]*\bdata-width\s*=\s*["'](?P<width>\d+)["'][^>]*\bdata-height\s*=\s*["'](?P<height>\d+)["']"#,
    )
    .expect("constant Prado pyramid pattern")
});

pub(super) fn contains_pyramid(contents: &[u8]) -> bool {
    PYRAMID.is_match(contents)
}

pub(super) fn load_pyramid(
    _: &DiscoveryContext<'_>,
    resource: DiscoveryResource<'_>,
) -> Result<DiscoveryStep, DiscoveryError> {
    let captures = PYRAMID
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
        size: Size { width, height },
        base_url: Some(resolve_relative(resource.final_uri(), origin)),
    };
    Ok(DiscoveryStep::Complete(catalog_from_dzi(
        resource.final_uri(),
        [image],
    )?))
}
