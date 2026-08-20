//! Transitional adapter between legacy dezoomers and the pure core registry.
//!
//! The adapter is deliberately outside `core`: it knows the legacy
//! `Dezoomer`/`TileProvider` traits, while the catalog it produces contains
//! only neutral values.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::ZoomError;
use crate::core::{
    AdaptiveProgram, CatalogEntry, DeferredImage, Dimensions, DiscoveryDiagnostic, DiscoveryError,
    DiscoveryInput, FormatHandler, FormatSession, ImageCatalog, ImageDescriptor, KnownTilePlan,
    LevelDescriptor, Point, Priority, ProcessingRecipe, Provenance, ProvenanceStep, Region,
    Registry, RequestRequirement, RequestSpec, ResourceOutcome, ResourcePurpose, ResourceRequest,
    SessionStep, StableId, TileId, TileObservation, TileProgram, TileRole, TileSpec,
};
use crate::dezoomer::{
    Dezoomer, DezoomerError, DezoomerInput, Images, PageContents, PostProcessFn, ResolvedImage,
    TileFetchResult, TileProvider, TileReference, Vec2d, ZoomLevel, ZoomableImage,
};

type LegacyFactory = fn() -> Box<dyn Dezoomer + Send>;

/// Registers one legacy format as a pure-core [`FormatHandler`].
#[derive(Clone, Copy)]
pub struct LegacyFormatHandler {
    id: &'static str,
    factory: LegacyFactory,
}

impl LegacyFormatHandler {
    #[must_use]
    pub const fn new(id: &'static str, factory: LegacyFactory) -> Self {
        Self { id, factory }
    }
}

impl FormatHandler for LegacyFormatHandler {
    fn id(&self) -> &'static str {
        self.id
    }

    fn start(&self, _input: &DiscoveryInput) -> Box<dyn FormatSession> {
        Box::new(LegacyFormatSession {
            id: self.id,
            dezoomer: (self.factory)(),
            pending_uri: None,
            first_request: true,
        })
    }
}

struct LegacyFormatSession {
    id: &'static str,
    dezoomer: Box<dyn Dezoomer + Send>,
    pending_uri: Option<String>,
    first_request: bool,
}

impl FormatSession for LegacyFormatSession {
    fn start(&mut self, input: &DiscoveryInput) -> Result<SessionStep, DiscoveryError> {
        self.advance(&DezoomerInput {
            uri: input.uri.clone(),
            contents: PageContents::Unknown,
        })
    }

    fn provide(&mut self, resource: &ResourceOutcome) -> Result<SessionStep, DiscoveryError> {
        let uri = self.pending_uri.take().ok_or_else(|| {
            DiscoveryError::Session(format!(
                "legacy format '{}' was not waiting for data",
                self.id
            ))
        })?;
        let contents = match resource {
            ResourceOutcome::Response(response) => PageContents::Success(response.bytes.clone()),
            ResourceOutcome::Failure(failure) => PageContents::Error(ZoomError::Io {
                source: std::io::Error::other(failure.message.clone()),
            }),
        };
        self.advance(&DezoomerInput { uri, contents })
    }
}

impl LegacyFormatSession {
    fn advance(&mut self, input: &DezoomerInput) -> Result<SessionStep, DiscoveryError> {
        match self.dezoomer.images(input) {
            Ok(images) => legacy_images_to_catalog(self.id, images).map(SessionStep::Complete),
            Err(DezoomerError::NeedsData { uri }) => {
                self.pending_uri = Some(uri.clone());
                let purpose = if self.first_request {
                    self.first_request = false;
                    ResourcePurpose::InitialMetadata
                } else {
                    ResourcePurpose::Metadata
                };
                Ok(SessionStep::Need(ResourceRequest::new(uri, purpose)))
            }
            Err(error) => Ok(SessionStep::Reject(DiscoveryDiagnostic::from(
                error.to_string(),
            ))),
        }
    }
}

/// Construct a registry containing every existing legacy format except the
/// recursive `AutoDezoomer`. Priorities reproduce the current registration
/// order while making it explicit and deterministic.
#[must_use]
pub fn legacy_registry() -> Registry {
    let mut registry = Registry::new();
    registry.register_format(
        "custom",
        Priority(10),
        Arc::new(LegacyFormatHandler::new("custom", custom)),
    );
    registry.register_format(
        "google_arts_and_culture",
        Priority(20),
        Arc::new(LegacyFormatHandler::new(
            "google_arts_and_culture",
            google_arts,
        )),
    );
    registry.register_format(
        "zoomify",
        Priority(30),
        Arc::new(LegacyFormatHandler::new("zoomify", zoomify)),
    );
    registry.register_format(
        "iiif",
        Priority(40),
        Arc::new(LegacyFormatHandler::new("iiif", iiif)),
    );
    registry.register_format(
        "deepzoom",
        Priority(50),
        Arc::new(LegacyFormatHandler::new("deepzoom", deepzoom)),
    );
    registry.register_format(
        "generic",
        Priority(60),
        Arc::new(LegacyFormatHandler::new("generic", generic)),
    );
    registry.register_format(
        "krpano",
        Priority(70),
        Arc::new(LegacyFormatHandler::new("krpano", krpano)),
    );
    registry.register_format(
        "IIPImage",
        Priority(80),
        Arc::new(LegacyFormatHandler::new("IIPImage", iipimage)),
    );
    registry.register_format(
        "nypl",
        Priority(90),
        Arc::new(LegacyFormatHandler::new("nypl", nypl)),
    );
    registry.register_format(
        "bulk_text",
        Priority(100),
        Arc::new(LegacyFormatHandler::new("bulk_text", bulk_text)),
    );
    registry
}

/// Construct the pure registry corresponding to a CLI format selection.
///
/// # Errors
///
/// Returns [`CatalogAdapterError::UnknownFormat`] for an unregistered ID.
pub fn legacy_registry_for(format_id: &str) -> Result<Registry, CatalogAdapterError> {
    if format_id == "auto" {
        return Ok(legacy_registry());
    }

    let (priority, handler): (i32, LegacyFormatHandler) = match format_id {
        "custom" => (10, LegacyFormatHandler::new("custom", custom)),
        "google_arts_and_culture" => (
            20,
            LegacyFormatHandler::new("google_arts_and_culture", google_arts),
        ),
        "zoomify" => (30, LegacyFormatHandler::new("zoomify", zoomify)),
        "iiif" => (40, LegacyFormatHandler::new("iiif", iiif)),
        "deepzoom" => (50, LegacyFormatHandler::new("deepzoom", deepzoom)),
        "generic" => (60, LegacyFormatHandler::new("generic", generic)),
        "krpano" => (70, LegacyFormatHandler::new("krpano", krpano)),
        "IIPImage" => (80, LegacyFormatHandler::new("IIPImage", iipimage)),
        "nypl" => (90, LegacyFormatHandler::new("nypl", nypl)),
        "bulk_text" => (100, LegacyFormatHandler::new("bulk_text", bulk_text)),
        _ => {
            return Err(CatalogAdapterError::UnknownFormat {
                id: format_id.to_owned(),
            });
        }
    };
    let mut registry = Registry::new();
    registry.register_format(format_id.to_owned(), Priority(priority), Arc::new(handler));
    Ok(registry)
}

fn custom() -> Box<dyn Dezoomer + Send> {
    Box::new(crate::custom_yaml::CustomDezoomer)
}
fn google_arts() -> Box<dyn Dezoomer + Send> {
    Box::new(crate::google_arts_and_culture::GAPDezoomer::default())
}
fn zoomify() -> Box<dyn Dezoomer + Send> {
    Box::new(crate::zoomify::ZoomifyDezoomer)
}
fn iiif() -> Box<dyn Dezoomer + Send> {
    Box::new(crate::iiif::IIIF)
}
fn deepzoom() -> Box<dyn Dezoomer + Send> {
    Box::new(crate::dzi::DziDezoomer)
}
fn generic() -> Box<dyn Dezoomer + Send> {
    Box::new(crate::generic::GenericDezoomer)
}
fn krpano() -> Box<dyn Dezoomer + Send> {
    Box::new(crate::krpano::KrpanoDezoomer::default())
}
fn iipimage() -> Box<dyn Dezoomer + Send> {
    Box::new(crate::iipimage::IIPImage)
}
fn nypl() -> Box<dyn Dezoomer + Send> {
    Box::new(crate::nypl::NYPLImage)
}
fn bulk_text() -> Box<dyn Dezoomer + Send> {
    Box::new(crate::bulk_text::BulkTextDezoomer)
}

/// Convert the legacy image collection into a neutral immutable catalog.
///
/// # Errors
///
/// Returns a discovery error when a legacy level cannot produce a valid,
/// deterministic neutral tile plan.
pub fn legacy_images_to_catalog(
    format_id: &str,
    images: Images,
) -> Result<ImageCatalog, DiscoveryError> {
    let entries = images
        .into_iter()
        .enumerate()
        .map(|(image_index, image)| legacy_image_to_entry(format_id, image_index, image))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ImageCatalog::new(entries))
}

fn legacy_image_to_entry(
    format_id: &str,
    image_index: usize,
    image: ZoomableImage,
) -> Result<CatalogEntry, DiscoveryError> {
    let image_id = StableId::new(format!("{format_id}:image:{image_index}"));
    let provenance = legacy_provenance(format_id);
    match image {
        ZoomableImage::Url(url) => Ok(CatalogEntry::Deferred(DeferredImage {
            id: image_id,
            uri: url.url,
            title: url.title,
            provenance,
            warnings: Vec::new(),
        })),
        ZoomableImage::Resolved(image) => {
            let title = image.title().map(str::to_owned);
            let mut levels = image
                .into_zoom_levels()
                .into_iter()
                .enumerate()
                .map(|(level_index, level)| {
                    legacy_level_to_descriptor(format_id, image_index, level_index, level)
                })
                .collect::<Result<Vec<_>, _>>()?;
            levels.sort_by_key(|level| {
                (
                    level.dimensions.is_none(),
                    level.dimensions.map_or(0, Dimensions::area),
                    level.id.as_str().to_owned(),
                )
            });
            let dimensions = levels
                .iter()
                .filter_map(|level| level.dimensions)
                .max_by_key(|dimensions| dimensions.area());
            let title = title.or_else(|| levels.iter().find_map(|level| level.title.clone()));
            Ok(CatalogEntry::Resolved(ImageDescriptor {
                id: image_id,
                title,
                dimensions,
                format: Some(format_id.to_owned()),
                levels,
                provenance,
                warnings: Vec::new(),
            }))
        }
    }
}

fn legacy_level_to_descriptor(
    format_id: &str,
    image_index: usize,
    level_index: usize,
    mut level: ZoomLevel,
) -> Result<LevelDescriptor, DiscoveryError> {
    let id = StableId::new(format!(
        "{format_id}:image:{image_index}:level:{level_index}"
    ));
    let dimensions = level.size_hint().map(dimensions_from_vec);
    let tile_size = level.tile_size_hint().map(dimensions_from_vec);
    let title = level.title();
    let scale_factor = level.scale_factor_hint();
    let has_overlapping_tiles = level.has_overlapping_tiles();
    let headers = ordered_headers(level.http_headers());
    let processing = processing_recipe(format_id);
    let program = if format_id == "generic" {
        let description = level
            .name()
            .strip_prefix("Generic image with template ")
            .map_or_else(|| level.name(), str::to_owned);
        TileProgram::Adaptive {
            id: id.clone(),
            description,
        }
    } else {
        let tile_references = level.next_tiles(None);
        let specs: Vec<_> = tile_references
            .into_iter()
            .enumerate()
            .map(|(tile_index, tile)| {
                legacy_tile_to_spec(
                    id.clone(),
                    tile_index,
                    tile,
                    tile_size,
                    &headers,
                    processing,
                )
            })
            .collect();
        TileProgram::Known(
            KnownTilePlan::new(specs)
                .map_err(|error| DiscoveryError::Session(error.to_string()))?,
        )
    };
    Ok(LevelDescriptor {
        id,
        title,
        dimensions,
        tile_size,
        scale_factor,
        has_overlapping_tiles,
        program,
        provenance: legacy_provenance(format_id),
        warnings: Vec::new(),
    })
}

fn legacy_tile_to_spec(
    level_id: StableId,
    tile_index: usize,
    tile: TileReference,
    tile_size: Option<Dimensions>,
    headers: &[RequestRequirement],
    processing: ProcessingRecipe,
) -> TileSpec {
    let origin = Point::new(tile.position.x, tile.position.y);
    let region = tile_size.map_or_else(
        || Region::new(origin, Dimensions::default()),
        |size| Region::new(origin, size),
    );
    TileSpec {
        id: TileId::new(level_id, TileRole::Output, tile_index as u64),
        request: RequestSpec::with_requirements(tile.url, headers.iter().cloned()),
        source_region: region,
        destination_region: region,
        expected_size: tile_size,
        processing,
        role: TileRole::Output,
    }
}

fn ordered_headers(headers: HashMap<String, String>) -> Vec<RequestRequirement> {
    let mut headers = headers.into_iter().collect::<Vec<_>>();
    headers.sort_unstable();
    headers
        .into_iter()
        .map(|(name, value)| RequestRequirement::Header { name, value })
        .collect()
}

fn processing_recipe(format_id: &str) -> ProcessingRecipe {
    if format_id == "google_arts_and_culture" {
        ProcessingRecipe::GoogleArtsDecrypt
    } else {
        ProcessingRecipe::None
    }
}

fn dimensions_from_vec(size: Vec2d) -> Dimensions {
    Dimensions::new(size.x, size.y)
}

fn vec_from_dimensions(size: Dimensions) -> Vec2d {
    Vec2d {
        x: size.width,
        y: size.height,
    }
}

fn legacy_provenance(format_id: &str) -> Provenance {
    Provenance::new([ProvenanceStep::new(
        StableId::new(format!("format:{format_id}")),
        format!("Adapted legacy format '{format_id}'"),
    )])
}

/// Conversion failures while presenting a neutral catalog to legacy callers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogAdapterError {
    UnsupportedAdaptive { level_id: StableId },
    UnknownFormat { id: String },
}

impl fmt::Display for CatalogAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedAdaptive { level_id } => {
                write!(
                    f,
                    "legacy adapter cannot present adaptive level '{level_id}'"
                )
            }
            Self::UnknownFormat { id } => write!(f, "unknown format '{id}'"),
        }
    }
}

impl std::error::Error for CatalogAdapterError {}

/// Convert a neutral catalog back into the existing `Images` presentation.
///
/// Known plans retain their canonical request order and all existing level
/// hints. Adaptive programs intentionally return an explicit error until the
/// native driver consumes them directly.
///
/// # Errors
///
/// Reserved for catalog features that cannot be represented by a compatibility
/// provider.
pub fn catalog_to_legacy_images(catalog: ImageCatalog) -> Result<Images, CatalogAdapterError> {
    let images = catalog
        .into_entries()
        .into_iter()
        .map(catalog_entry_to_legacy_image)
        .collect::<Vec<_>>();
    Ok(images.into())
}

fn catalog_entry_to_legacy_image(entry: CatalogEntry) -> ZoomableImage {
    match entry {
        CatalogEntry::Deferred(image) => ZoomableImage::Url(crate::dezoomer::ImageUrl {
            url: image.uri,
            title: image.title,
        }),
        CatalogEntry::Resolved(image) => {
            let levels = image
                .levels
                .into_iter()
                .map(descriptor_to_legacy_level)
                .collect();
            ZoomableImage::Resolved(ResolvedImage::new(levels, image.title))
        }
    }
}

fn descriptor_to_legacy_level(level: LevelDescriptor) -> ZoomLevel {
    match level.program {
        TileProgram::Known(plan) => Box::new(DescriptorTileProvider {
            id: level.id,
            title: level.title,
            dimensions: level.dimensions,
            tile_size: level.tile_size,
            scale_factor: level.scale_factor,
            has_overlapping_tiles: level.has_overlapping_tiles,
            specs: plan.specs().to_vec(),
            processing: plan
                .specs()
                .first()
                .map_or(ProcessingRecipe::None, |spec| spec.processing),
        }),
        TileProgram::Adaptive { id, description } => Box::new(AdaptiveTileProvider {
            program: AdaptiveProgram::new(id.clone(), description.clone()),
            id,
            description,
            pending: Vec::new(),
            image_size: None,
        }),
    }
}

struct AdaptiveTileProvider {
    program: AdaptiveProgram,
    id: StableId,
    description: String,
    pending: Vec<TileSpec>,
    image_size: Option<Dimensions>,
}

impl fmt::Debug for AdaptiveTileProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Adaptive level {}", self.id)
    }
}

impl TileProvider for AdaptiveTileProvider {
    fn next_tiles(&mut self, previous: Option<TileFetchResult>) -> Vec<TileReference> {
        if let Some(previous) = previous
            && self
                .pending
                .first()
                .is_some_and(|spec| spec.role == TileRole::Probe)
        {
            let Some(spec) = self.pending.first() else {
                return Vec::new();
            };
            let observation = if previous.is_success() {
                let Some(size) = previous.tile_size else {
                    return Vec::new();
                };
                TileObservation::success(spec.id.clone(), dimensions_from_vec(size))
            } else {
                TileObservation::failure(spec.id.clone())
            };
            if self.program.submit([observation]).is_err() {
                return Vec::new();
            }
        }
        self.pending.clear();
        self.image_size = self.program.image_size();
        let Ok(Some(batch)) = self.program.take_ready(usize::MAX) else {
            return Vec::new();
        };
        let references = batch
            .iter()
            .map(|spec| TileReference {
                url: spec.request.uri.clone(),
                position: Vec2d {
                    x: spec.destination_region.origin.x,
                    y: spec.destination_region.origin.y,
                },
            })
            .collect();
        self.pending = batch;
        references
    }

    fn name(&self) -> String {
        format!("Generic image with template {}", self.description)
    }

    fn size_hint(&self) -> Option<Vec2d> {
        self.image_size.map(vec_from_dimensions)
    }
}

struct DescriptorTileProvider {
    id: StableId,
    title: Option<String>,
    dimensions: Option<Dimensions>,
    tile_size: Option<Dimensions>,
    scale_factor: Option<u32>,
    has_overlapping_tiles: bool,
    specs: Vec<TileSpec>,
    processing: ProcessingRecipe,
}

impl fmt::Debug for DescriptorTileProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Neutral level {}", self.id)
    }
}

impl TileProvider for DescriptorTileProvider {
    fn next_tiles(&mut self, previous: Option<TileFetchResult>) -> Vec<TileReference> {
        if previous.is_some() {
            return Vec::new();
        }
        self.specs
            .iter()
            .map(|spec| TileReference {
                url: spec.request.uri.clone(),
                position: Vec2d {
                    x: spec.destination_region.origin.x,
                    y: spec.destination_region.origin.y,
                },
            })
            .collect()
    }

    fn title(&self) -> Option<String> {
        self.title.clone()
    }

    fn size_hint(&self) -> Option<Vec2d> {
        self.dimensions.map(vec_from_dimensions)
    }

    fn tile_count_hint(&self) -> Option<u32> {
        self.specs.len().try_into().ok()
    }

    fn tile_size_hint(&self) -> Option<Vec2d> {
        self.tile_size.map(vec_from_dimensions)
    }

    fn scale_factor_hint(&self) -> Option<u32> {
        self.scale_factor
    }

    fn has_overlapping_tiles(&self) -> bool {
        self.has_overlapping_tiles
    }

    fn http_headers(&self) -> HashMap<String, String> {
        self.specs
            .first()
            .into_iter()
            .flat_map(|spec| spec.request.requirements.iter())
            .filter_map(|requirement| match requirement {
                RequestRequirement::Header { name, value } => Some((name.clone(), value.clone())),
                RequestRequirement::AcceptContentType(_) | RequestRequirement::Method(_) => None,
            })
            .collect()
    }

    fn post_process_fn(&self) -> PostProcessFn {
        match self.processing {
            ProcessingRecipe::None => PostProcessFn::None,
            ProcessingRecipe::GoogleArtsDecrypt => {
                PostProcessFn::Fn(crate::google_arts_and_culture::post_process_tile)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dezoomer::{TilesRect, single_level};

    #[derive(Debug)]
    struct FixedLevel;

    impl TilesRect for FixedLevel {
        fn size(&self) -> Vec2d {
            Vec2d { x: 5, y: 3 }
        }

        fn tile_size(&self) -> Vec2d {
            Vec2d { x: 3, y: 2 }
        }

        fn tile_url(&self, position: Vec2d) -> String {
            format!("memory://{}/{}", position.x, position.y)
        }

        fn title(&self) -> Option<String> {
            Some("fixed".into())
        }
    }

    #[test]
    fn fixed_legacy_provider_becomes_replayable_neutral_plan() {
        let catalog = legacy_images_to_catalog(
            "fixture",
            ResolvedImage::new(single_level(FixedLevel), Some("image".into())).into(),
        )
        .unwrap();
        let CatalogEntry::Resolved(image) = &catalog.entries()[0] else {
            panic!("fixture image should be resolved");
        };
        assert_eq!(image.levels.len(), 1);
        let TileProgram::Known(plan) = &image.levels[0].program else {
            panic!("fixed provider should become known plan");
        };
        assert_eq!(plan.specs().len(), 4);
        assert_eq!(plan.specs()[0].request.uri, "memory://0/0");

        let images = catalog_to_legacy_images(catalog).unwrap();
        let ZoomableImage::Resolved(image) = images.into_iter().next().unwrap() else {
            panic!("round trip should stay resolved");
        };
        let mut levels = image.into_zoom_levels();
        let mut level = levels.remove(0);
        assert_eq!(level.size_hint(), Some(Vec2d { x: 5, y: 3 }));
        assert_eq!(level.next_tiles(None).len(), 4);
    }

    #[test]
    fn registry_has_unique_explicit_legacy_priorities() {
        legacy_registry().validate().unwrap();
    }
}
