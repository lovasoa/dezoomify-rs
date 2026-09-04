//! Pure discovery for cached `ArcGIS` REST `MapServer` services.

use std::sync::Arc;

use serde::Deserialize;
use url::Url;

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryError, DiscoveryMatch, Grid, ImageCatalog,
    ImageDescriptor, LevelDescriptor, Request, StableId,
};

pub const SPEC: DezoomerSpec = DezoomerSpec::new(
    "arcgis",
    &[
        DiscoveryMatch::UrlPredicate(is_arcgis_url).map_url(metadata_url),
        DiscoveryMatch::Any.extract(catalog),
    ],
)
.recognizing(is_arcgis_url, "not an ArcGIS MapServer URL")
.preferring(is_arcgis_url);

fn is_arcgis_url(uri: &str) -> bool {
    is_map_server_url(uri) || basemap_url(uri).is_some()
}

fn is_map_server_url(uri: &str) -> bool {
    Url::parse(uri).is_ok_and(|url| {
        url.path()
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case("MapServer"))
    })
}

fn metadata_url(input: &str) -> Result<Request, DiscoveryError> {
    let input = basemap_url(input).unwrap_or_else(|| input.to_owned());
    let service = service_url(&input)?;
    let parameters = tile_parameters(&input)?;
    let query = if parameters.is_empty() {
        "f=json".to_owned()
    } else {
        format!("{parameters}&f=json")
    };
    Ok(Request::new(format!("{service}?{query}")))
}

fn basemap_url(input: &str) -> Option<String> {
    let url = Url::parse(input).ok()?;
    let value = url
        .query_pairs()
        .find_map(|(name, value)| name.eq_ignore_ascii_case("basemapUrl").then_some(value))?;
    let candidate = url.join(&value).ok()?.to_string();
    is_map_server_url(&candidate).then_some(candidate)
}

fn service_url(input: &str) -> Result<String, DiscoveryError> {
    let url = Url::parse(input)
        .map_err(|_| DiscoveryError::Session("invalid ArcGIS MapServer URL".into()))?;
    let path = url
        .path()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    if !path.eq_ignore_ascii_case("MapServer") {
        return Err(DiscoveryError::Session(
            "expected an ArcGIS MapServer URL".into(),
        ));
    }
    let before_query = input.split_once(['?', '#']).map_or(input, |(path, _)| path);
    Ok(before_query.trim_end_matches('/').to_owned())
}

fn tile_parameters(input: &str) -> Result<String, DiscoveryError> {
    let url = Url::parse(input)
        .map_err(|_| DiscoveryError::Session("invalid ArcGIS MapServer URL".into()))?;
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in url.query_pairs() {
        if !name.eq_ignore_ascii_case("f") {
            serializer.append_pair(&name, &value);
        }
    }
    Ok(serializer.finish())
}

fn catalog(url: &str, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let mut metadata: Metadata = serde_json::from_slice(bytes).map_err(|error| {
        DiscoveryError::Session(format!(
            "unable to parse ArcGIS MapServer metadata: {error}"
        ))
    })?;
    let title = metadata
        .map_name
        .take()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    let (tile_info, extent) = validate_metadata(metadata)?;
    let service: Arc<str> = service_url(url)?.into();
    let parameters: Arc<str> = tile_parameters(url)?.into();
    let levels = build_levels(tile_info, &extent, &service, &parameters)?;
    if levels.is_empty() {
        return Err(DiscoveryError::Session(
            "ArcGIS MapServer has no LODs".into(),
        ));
    }
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("arcgis:image"),
        title,
        format: StableId::new("arcgis"),
        levels,
        ..Default::default()
    })]))
}

fn validate_metadata(metadata: Metadata) -> Result<(TileInfo, Extent), DiscoveryError> {
    if metadata.service_type.as_deref() != Some("MapServer") {
        return Err(DiscoveryError::Session(
            "ArcGIS service is not a MapServer".into(),
        ));
    }
    if !metadata.single_fused_map_cache {
        return Err(DiscoveryError::Session(
            "ArcGIS MapServer does not provide a fused tile cache".into(),
        ));
    }
    let tile_info = metadata.tile_info.ok_or_else(|| {
        DiscoveryError::Session("ArcGIS MapServer is missing tile metadata".into())
    })?;
    let extent = metadata.full_extent.ok_or_else(|| {
        DiscoveryError::Session("ArcGIS MapServer is missing full extent metadata".into())
    })?;
    if !matching_spatial_references(&tile_info.spatial_reference, &extent.spatial_reference) {
        return Err(DiscoveryError::Session(
            "ArcGIS tile cache and extent use different spatial references".into(),
        ));
    }
    if tile_info.cols == 0 || tile_info.rows == 0 || tile_info.cols != tile_info.rows {
        return Err(DiscoveryError::Session(
            "ArcGIS MapServer must use square cached tiles".into(),
        ));
    }
    if !extent.xmin.is_finite()
        || !extent.ymin.is_finite()
        || !extent.xmax.is_finite()
        || !extent.ymax.is_finite()
        || extent.xmin > extent.xmax
        || extent.ymin > extent.ymax
    {
        return Err(DiscoveryError::Session(
            "invalid ArcGIS MapServer extent".into(),
        ));
    }
    Ok((tile_info, extent))
}

fn build_levels(
    tile_info: TileInfo,
    extent: &Extent,
    service: &Arc<str>,
    parameters: &Arc<str>,
) -> Result<Vec<LevelDescriptor>, DiscoveryError> {
    let tile_width = tile_info.cols;
    let tile_height = tile_info.rows;
    let origin_x = tile_info.origin.x;
    let origin_y = tile_info.origin.y;
    let xmin = extent.xmin;
    let ymin = extent.ymin;
    let xmax = extent.xmax;
    let ymax = extent.ymax;
    tile_info
        .lods
        .into_iter()
        .enumerate()
        .map(|(ordinal, lod)| {
            if !lod.resolution.is_finite() || lod.resolution <= 0.0 {
                return Err(DiscoveryError::Session(
                    "invalid ArcGIS LOD resolution".into(),
                ));
            }
            let span = f64::from(tile_width) * lod.resolution;
            let min_column = floor_index((xmin - origin_x) / span)?;
            let max_column = floor_index((xmax - origin_x) / span)?;
            let min_row = floor_index((origin_y - ymax) / span)?;
            let max_row = floor_index((origin_y - ymin) / span)?;
            let columns = count_between(min_column, max_column)?;
            let rows = count_between(min_row, max_row)?;
            let width = columns
                .checked_mul(u64::from(tile_width))
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| DiscoveryError::Session("ArcGIS image width is too large".into()))?;
            let height = rows
                .checked_mul(u64::from(tile_height))
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    DiscoveryError::Session("ArcGIS image height is too large".into())
                })?;
            let service = Arc::clone(service);
            let parameters = Arc::clone(parameters);
            let level = lod.level;
            let source = Grid::with_requests(
                StableId::new(format!("arcgis:{ordinal}")),
                Vec2d {
                    x: width,
                    y: height,
                },
                Vec2d::square(tile_width),
                Vec2d::default(),
                move |tile| {
                    let column = min_column + i64::from(tile.coord.column);
                    let row = min_row + i64::from(tile.coord.row);
                    let uri = format!("{service}/tile/{level}/{row}/{column}");
                    if parameters.is_empty() {
                        Request::new(uri)
                    } else {
                        Request::new(format!("{uri}?{parameters}"))
                    }
                },
            )
            .map_err(|error| DiscoveryError::Session(format!("invalid ArcGIS grid: {error}")))?;
            Ok(LevelDescriptor::new(source).with_title(Some(format!("ArcGIS level {level}"))))
        })
        .collect()
}

fn matching_spatial_references(first: &SpatialReference, second: &SpatialReference) -> bool {
    reference_id(first)
        .zip(reference_id(second))
        .is_some_and(|(first, second)| first == second)
}

fn reference_id(reference: &SpatialReference) -> Option<String> {
    reference.latest_wkid.or(reference.wkid).map_or_else(
        || reference.wkt.clone(),
        |id| Some(canonical_wkid(id).to_string()),
    )
}

fn canonical_wkid(id: i64) -> i64 {
    match id {
        // ArcGIS uses all of these identifiers for Web Mercator auxiliary sphere.
        3857 | 102_100 | 102_113 | 900_913 | 3785 => 3857,
        _ => id,
    }
}

fn floor_index(value: f64) -> Result<i64, DiscoveryError> {
    let value = value.floor();
    value
        .is_finite()
        .then(|| value.to_string().parse::<i64>())
        .transpose()
        .map_err(|_| DiscoveryError::Session("ArcGIS tile coordinate is out of range".into()))?
        .ok_or_else(|| DiscoveryError::Session("ArcGIS tile coordinate is out of range".into()))
}

fn count_between(minimum: i64, maximum: i64) -> Result<u64, DiscoveryError> {
    if maximum < minimum {
        return Err(DiscoveryError::Session(
            "ArcGIS extent is outside its tile cache".into(),
        ));
    }
    maximum
        .checked_sub(minimum)
        .and_then(|value| value.checked_add(1))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| DiscoveryError::Session("ArcGIS tile range is too large".into()))
}

#[derive(Debug, Deserialize)]
struct Metadata {
    #[serde(rename = "type")]
    service_type: Option<String>,
    #[serde(rename = "singleFusedMapCache", default)]
    single_fused_map_cache: bool,
    #[serde(rename = "mapName", default)]
    map_name: Option<String>,
    #[serde(rename = "tileInfo")]
    tile_info: Option<TileInfo>,
    #[serde(rename = "fullExtent")]
    full_extent: Option<Extent>,
}

#[derive(Debug, Deserialize)]
struct TileInfo {
    rows: u32,
    cols: u32,
    origin: Point,
    #[serde(rename = "spatialReference")]
    spatial_reference: SpatialReference,
    lods: Vec<Lod>,
}

#[derive(Debug, Deserialize)]
struct Lod {
    level: u32,
    resolution: f64,
}

#[derive(Debug, Deserialize)]
struct Extent {
    xmin: f64,
    ymin: f64,
    xmax: f64,
    ymax: f64,
    #[serde(rename = "spatialReference")]
    spatial_reference: SpatialReference,
}

#[derive(Debug, Deserialize)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Debug, Deserialize)]
struct SpatialReference {
    #[serde(rename = "latestWkid")]
    latest_wkid: Option<i64>,
    wkid: Option<i64>,
    wkt: Option<String>,
}
