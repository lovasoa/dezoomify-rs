#![deny(clippy::cognitive_complexity)]
#![deny(clippy::too_many_lines)]
#![deny(clippy::missing_errors_doc)]
#![deny(clippy::missing_panics_doc)]
#![deny(clippy::pedantic)]

use std::env::current_dir;

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::{fs, io};

use log::{debug, error, info, warn};

pub use arguments::Arguments;
pub use binary_display::{BinaryDisplay, display_bytes};
pub use dezoomify_core::Vec2d;
pub use errors::ZoomError;
use network::client;
use output_file::get_outname;
use tile::Tile;

use crate::encoder::SourceLevel;
use crate::encoder::tile_buffer::TileBuffer;

use crate::native::NativeDiscoveryDriver;
use crate::output_file::reserve_output_file;
use dezoomify_core::core::{
    CatalogEntry, DeferredImage, ImageCatalog, ImageDescriptor, LevelDescriptor, LevelPlan,
};

mod arguments;
mod binary_display;

pub(crate) mod download_state;
mod encoder;
mod errors;
mod native;
mod network;
mod output_file;
mod registry;
mod throttler;
pub mod tile;

fn stdin_line() -> Result<String, ZoomError> {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let first_line = lines.next().ok_or_else(|| {
        let err_msg = "Encountered end of standard input while reading a line";
        io::Error::new(io::ErrorKind::UnexpectedEof, err_msg)
    })?;
    Ok(first_line?)
}

/// Process an input URI to extract zoomable images
async fn get_images_from_uri(
    args: &Arguments,
    resolver: &NativeDiscoveryDriver,
    uri: &str,
) -> Result<ImageCatalog, ZoomError> {
    let registry =
        crate::registry::registry_for_cli(args.dezoomer_name(), uri).ok_or_else(|| {
            ZoomError::NoSuchDezoomer {
                name: args.dezoomer_name().to_owned(),
            }
        })?;
    discover_images(resolver, &registry, uri)
        .await
        .map_err(|message| ZoomError::Dezoomer { message })
}

/// Validates a user input line as a level index
fn parse_level_index(input: &str, max_index: usize) -> Option<usize> {
    input.parse::<usize>().ok().filter(|&idx| idx < max_index)
}

/// Gets the actual level index to use, handling out-of-bounds requests
fn resolve_level_index(requested: usize, available_count: usize) -> usize {
    if requested < available_count {
        requested
    } else {
        available_count - 1
    }
}

/// Gets the actual image index to use, handling out-of-bounds requests
fn resolve_image_index(requested: usize, available_count: usize) -> usize {
    if requested < available_count {
        requested
    } else {
        available_count - 1
    }
}

/// Finds the position of a level with the specified size hint
fn find_level_with_size(levels: &[LevelDescriptor], target_size: Vec2d) -> Option<usize> {
    levels.iter().position(|l| l.size == Some(target_size))
}

/// An interactive level picker
fn level_picker(mut levels: Vec<LevelDescriptor>) -> Result<LevelDescriptor, ZoomError> {
    println!("Found the following zoom levels:");
    for (i, level) in levels.iter().enumerate() {
        println!("{i: >2}. {}", level.display_label());
    }
    loop {
        println!("Which level do you want to download? ");
        let line = stdin_line()?;
        if let Some(idx) = parse_level_index(&line, levels.len()) {
            return Ok(levels.swap_remove(idx));
        }
        error!("'{line}' is not a valid level number");
    }
}

fn choose_level(
    mut levels: Vec<LevelDescriptor>,
    args: &Arguments,
) -> Result<LevelDescriptor, ZoomError> {
    match levels.len() {
        0 => Err(ZoomError::NoLevels),
        1 => Ok(levels.swap_remove(0)),
        _ => {
            if let Some(requested_level) = args.zoom_level {
                let actual_level = resolve_level_index(requested_level, levels.len());
                if actual_level == requested_level {
                    info!("Selected zoom level {requested_level} as requested");
                } else {
                    info!(
                        "Requested zoom level {requested_level} not available. Using last one ({actual_level})"
                    );
                }
                return Ok(levels.swap_remove(actual_level));
            }

            if let Some(best_size) = args.best_size(levels.iter().filter_map(|l| l.size))
                && let Some(pos) = find_level_with_size(&levels, best_size)
            {
                return Ok(levels.swap_remove(pos));
            }

            level_picker(levels)
        }
    }
}

/// An interactive image picker for when multiple images are available
fn image_picker(mut images: Vec<CatalogEntry>) -> Result<CatalogEntry, ZoomError> {
    println!("Found the following images:");
    for (i, image) in images.iter().enumerate() {
        let title =
            catalog_entry_title(image).map_or_else(|| format!("Image {}", i + 1), str::to_string);
        println!("{i: >2}. {title}");
    }
    loop {
        println!("Which image do you want to download? ");
        let line = stdin_line()?;
        if let Some(idx) = parse_level_index(&line, images.len()) {
            return Ok(images.swap_remove(idx));
        }
        error!("'{line}' is not a valid image number");
    }
}

/// Choose an image from multiple options (interactive or automatic)
fn choose_image(
    mut images: Vec<CatalogEntry>,
    args: &Arguments,
) -> Result<CatalogEntry, ZoomError> {
    match images.len() {
        0 => Err(ZoomError::NoLevels),
        1 => Ok(images.swap_remove(0)),
        _ => {
            if let Some(requested_index) = args.image_index {
                let actual_index = resolve_image_index(requested_index, images.len());
                if actual_index == requested_index {
                    info!("Selected image {requested_index} as requested");
                } else {
                    info!(
                        "Requested image index {requested_index} not available. Using last one ({actual_index})"
                    );
                }
                return Ok(images.swap_remove(actual_index));
            }

            // In bulk mode, automatically select the first image to avoid interactive prompts
            if args.is_bulk_mode() {
                info!("Bulk mode: automatically selecting first image (index 0)");
                return Ok(images.swap_remove(0));
            }

            // Interactive selection when no command line option is provided
            image_picker(images)
        }
    }
}

async fn resolve_selected_image(
    mut image: CatalogEntry,
    args: &Arguments,
    resolver: &NativeDiscoveryDriver,
) -> Result<ImageDescriptor, ZoomError> {
    loop {
        match image {
            CatalogEntry::Ready(image) => return Ok(image),
            CatalogEntry::Deferred(image_url) => {
                let images = resolve_deferred_images(image_url, resolver)
                    .await
                    .map_err(|message| ZoomError::Dezoomer { message })?;
                image = choose_image(images.into_entries(), args)?;
            }
        }
    }
}

async fn resolve_deferred_images(
    image_url: DeferredImage,
    resolver: &NativeDiscoveryDriver,
) -> Result<ImageCatalog, String> {
    let url = image_url.uri.clone();
    discover_images(resolver, &crate::registry::default_registry(&url), &url)
        .await
        .map(|catalog| inherit_deferred_context(catalog, &image_url))
}

fn inherit_deferred_context(catalog: ImageCatalog, parent: &DeferredImage) -> ImageCatalog {
    let parent_title = parent
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty());
    ImageCatalog::new(catalog.into_entries().into_iter().map(|mut entry| {
        let (title, provenance, warnings) = match entry {
            CatalogEntry::Ready(ref mut image) => {
                (&mut image.title, &mut image.provenance, &mut image.warnings)
            }
            CatalogEntry::Deferred(ref mut image) => {
                (&mut image.title, &mut image.provenance, &mut image.warnings)
            }
        };
        if title.as_deref().is_none_or(str::is_empty) {
            *title = parent_title.map(str::to_owned);
        }
        provenance.0.splice(0..0, parent.provenance.0.clone());
        warnings.splice(0..0, parent.warnings.clone());
        entry
    }))
}

async fn discover_images(
    driver: &NativeDiscoveryDriver,
    registry: &dezoomify_core::core::Registry,
    uri: &str,
) -> Result<ImageCatalog, String> {
    let catalog = driver
        .discover(registry, uri)
        .await
        .map_err(|error| error.to_string())?;
    Ok(catalog)
}

fn catalog_entry_title(entry: &CatalogEntry) -> Option<&str> {
    match entry {
        CatalogEntry::Ready(image) => image.title.as_deref(),
        CatalogEntry::Deferred(image) => image.title.as_deref(),
    }
}

/// Prepares the output file path for saving
fn prepare_output_path(
    outfile_arg: Option<&Path>,
    title: Option<&str>,
    base_dir: &Path,
    size_hint: Option<Vec2d>,
) -> Result<PathBuf, ZoomError> {
    let outname = get_outname(outfile_arg, title, base_dir, size_hint);
    let save_as = fs::canonicalize(outname.as_path()).unwrap_or_else(|_e| outname.clone());
    reserve_output_file(&save_as)?;
    Ok(save_as)
}

/// Creates a tile buffer for the given output path
fn create_tile_buffer(save_as: PathBuf, compression: u8) -> TileBuffer {
    TileBuffer::new(save_as, compression)
}

fn output_prefers_source_pyramid(path: &Path, args: &Arguments) -> bool {
    if args.has_level_specifying_args() || args.largest {
        return false;
    }
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("iiif" | "tif" | "tiff" | "zif")
    )
}

fn can_dezoomify_source_pyramid(path: &Path, args: &Arguments, levels: &[LevelDescriptor]) -> bool {
    output_prefers_source_pyramid(path, args)
        && largest_level_size(levels).is_some()
        && levels.iter().all(|level| {
            level.size.is_some() && level.tile_size.is_some() && !level.has_overlapping_tiles
        })
}

async fn dezoomify_source_pyramid(
    args: &Arguments,
    mut levels: Vec<LevelDescriptor>,
    tile_buffer: TileBuffer,
) -> Result<(), ZoomError> {
    let mut canvas = tile_buffer;
    let full_size = largest_level_size(&levels).ok_or(ZoomError::NoLevels)?;
    let base_scale_factor = levels
        .iter()
        .filter(|level| level.size == Some(full_size))
        .filter_map(|level| level.scale_factor)
        .filter(|&scale_factor| scale_factor > 0)
        .min()
        .unwrap_or(1);
    levels.sort_by_key(|level| std::cmp::Reverse(level_area(level.size)));

    let mut total_tiles = 0;
    let mut successful_tiles = 0;
    for (index, level) in levels.into_iter().enumerate() {
        let level_size = level.size.unwrap_or(full_size);
        let scale_factor =
            source_level_scale_factor(full_size, level_size, level.scale_factor, base_scale_factor);
        canvas
            .begin_level(SourceLevel {
                index,
                size: full_size,
                scale_factor,
                tile_size: level.tile_size,
                has_overlapping_tiles: level.has_overlapping_tiles,
            })
            .await?;
        let state = dezoomify_level_into_buffer(args, level, &mut canvas).await?;
        validate_download_success(&state)?;
        total_tiles += state.total_tiles;
        successful_tiles += state.successful_tiles;
    }

    finalize_canvas(&mut canvas).await?;
    if successful_tiles < total_tiles {
        Err(ZoomError::PartialDownload {
            successful_tiles,
            total_tiles,
            destination: canvas.destination().to_string_lossy().to_string(),
        })
    } else {
        Ok(())
    }
}

fn source_level_scale_factor(
    full_size: Vec2d,
    level_size: Vec2d,
    scale_factor: Option<u32>,
    base_scale_factor: u32,
) -> u32 {
    source_level_scale_factor_from_hint(full_size, level_size, scale_factor, base_scale_factor)
}

fn source_level_scale_factor_from_hint(
    full_size: Vec2d,
    level_size: Vec2d,
    scale_factor_hint: Option<u32>,
    base_scale_factor: u32,
) -> u32 {
    if let Some(scale_factor) = scale_factor_hint
        .filter(|&scale_factor| scale_factor > 0)
        .filter(|scale_factor| scale_factor % base_scale_factor == 0)
    {
        return (scale_factor / base_scale_factor).max(1);
    }
    full_size.x.div_ceil(level_size.x).max(1)
}

fn largest_level_size(levels: &[LevelDescriptor]) -> Option<Vec2d> {
    levels
        .iter()
        .filter_map(|level| level.size)
        .max_by_key(|size| level_area(Some(*size)))
}

fn level_area(size: Option<Vec2d>) -> u64 {
    size.map_or(0, |size| u64::from(size.x) * u64::from(size.y))
}

/// Downloads the image selected by `args` and returns its output path.
///
/// # Errors
///
/// Returns an error if the input cannot be resolved, no suitable level can be selected,
/// output setup fails, or the image cannot be downloaded and encoded.
pub async fn dezoomify(args: &Arguments) -> Result<PathBuf, ZoomError> {
    let uri = args.choose_input_uri()?;
    let http_client = client(args.headers(), args, Some(&uri))?;
    let resolver = NativeDiscoveryDriver::with_user_headers(
        http_client,
        crate::network::user_header_names(args.headers()),
    );
    debug!("Trying to locate a zoomable image...");
    let images = get_images_from_uri(args, &resolver, &uri).await?;
    debug!("Found {} zoomable images", images.len());
    let selected_image = choose_image(images.into_entries(), args)?;
    let resolved_image = resolve_selected_image(selected_image, args, &resolver).await?;
    debug!("Resolved {} image", resolved_image.format);
    for warning in &resolved_image.warnings {
        warn!("{warning}");
    }
    let title = resolved_image.title.clone();
    let zoom_levels = resolved_image.levels;

    let base_dir = current_dir()?;
    let output_file = args.output_file();
    let largest_size = largest_level_size(&zoom_levels);
    let source_pyramid_path = get_outname(
        output_file.as_deref(),
        title.as_deref(),
        &base_dir,
        largest_size,
    );

    if can_dezoomify_source_pyramid(&source_pyramid_path, args, &zoom_levels) {
        let save_as = prepare_output_path(
            output_file.as_deref(),
            title.as_deref(),
            &base_dir,
            largest_size,
        )?;
        let tile_buffer = create_tile_buffer(save_as.clone(), args.compression);
        info!("Dezooming source pyramid with {} levels", zoom_levels.len());
        dezoomify_source_pyramid(args, zoom_levels, tile_buffer).await?;
        Ok(save_as)
    } else {
        let zoom_level = choose_level(zoom_levels, args)?;
        let save_as = prepare_output_path(
            output_file.as_deref(),
            title.as_deref(),
            &base_dir,
            zoom_level.size,
        )?;
        let tile_buffer = create_tile_buffer(save_as.clone(), args.compression);
        info!(
            "Dezooming {}",
            zoom_level.title.clone().unwrap_or_else(|| "level".into())
        );
        dezoomify_level(args, zoom_level, tile_buffer).await?;
        Ok(save_as)
    }
}

/// Statistics for bulk processing
#[derive(Debug, Default)]
pub struct BulkStats {
    pub total_images: usize,
    pub successful_images: usize,
    pub failed_images: usize,
    pub partial_downloads: usize,
}

impl BulkStats {
    fn new() -> Self {
        Self::default()
    }

    fn record_success(&mut self) {
        self.successful_images += 1;
    }

    fn record_partial(&mut self) {
        self.partial_downloads += 1;
    }

    fn record_failure(&mut self) {
        self.failed_images += 1;
    }

    fn set_total(&mut self, total: usize) {
        self.total_images = total;
    }
}

/// Process every image discovered from a bulk input.
///
/// # Errors
///
/// Returns an error if the bulk source cannot be resolved or shared processing setup fails.
/// Failures for individual images are recorded in the returned statistics.
pub async fn process_bulk(args: &Arguments) -> Result<BulkStats, ZoomError> {
    use log::{debug, trace};

    debug!("Starting bulk processing mode");
    trace!("Bulk processing arguments: {args:?}");

    // Get the bulk file/URI from arguments
    let bulk_uri = args.bulk.as_ref().ok_or_else(|| ZoomError::NoBulkUrl {
        bulk_file_path: "No bulk source specified".to_string(),
    })?;

    debug!("Bulk source: {bulk_uri}");

    // Discover images from the bulk source.
    let http = client(std::iter::empty(), args, None)?;
    let resolver = NativeDiscoveryDriver::new(http);
    let registry =
        crate::registry::registry_for_cli(args.dezoomer_name(), bulk_uri).ok_or_else(|| {
            ZoomError::NoSuchDezoomer {
                name: args.dezoomer_name().to_owned(),
            }
        })?;
    let images = discover_images(&resolver, &registry, bulk_uri)
        .await
        .map_err(|message| ZoomError::Dezoomer { message })?;

    let mut stats = BulkStats::new();
    let base_dir = current_dir()?;

    stats.set_total(images.len());
    info!("Found {} images to process in bulk mode", images.len());
    debug!(
        "Images discovered: {:?}",
        images
            .entries()
            .iter()
            .map(|img| catalog_entry_title(img).unwrap_or("Untitled"))
            .collect::<Vec<_>>()
    );

    process_bulk_zoomable_images(
        images.into_entries(),
        args,
        &resolver,
        &mut stats,
        &base_dir,
    )
    .await?;

    // Log final statistics
    info!("Bulk processing complete!");
    info!("Total images: {}", stats.total_images);
    info!("Successfully downloaded: {}", stats.successful_images);
    info!("Partial downloads: {}", stats.partial_downloads);
    info!("Failed downloads: {}", stats.failed_images);

    debug!("Final bulk processing stats: {stats:?}");

    Ok(stats)
}

/// Resolve and process images without fetching deferred metadata ahead of time.
async fn process_bulk_zoomable_images(
    images: Vec<CatalogEntry>,
    args: &Arguments,
    resolver: &NativeDiscoveryDriver,
    stats: &mut BulkStats,
    base_dir: &Path,
) -> Result<(), ZoomError> {
    use std::collections::VecDeque;

    let bulk_outfile = args.bulk_output_file();
    let mut pending = VecDeque::from(images);
    let mut index = 0;

    while let Some(catalog_entry) = pending.pop_front() {
        let image_title = catalog_entry_title(&catalog_entry)
            .map_or_else(|| format!("Image_{}", index + 1), str::to_string);

        let resolved_image = match catalog_entry {
            CatalogEntry::Ready(image) => image,
            CatalogEntry::Deferred(image_url) => {
                match resolve_deferred_images(image_url, resolver).await {
                    Ok(images) if !images.is_empty() => {
                        let images = images.into_entries();
                        stats.total_images += images.len() - 1;
                        for image in images.into_iter().rev() {
                            pending.push_front(image);
                        }
                        continue;
                    }
                    Ok(_) => {
                        log::warn!(
                            "No images found for image {} ('{}')",
                            index + 1,
                            image_title
                        );
                        stats.record_failure();
                        index += 1;
                        continue;
                    }
                    Err(e) => {
                        log::warn!(
                            "Failed to resolve image {} ('{}'): {}",
                            index + 1,
                            image_title,
                            e
                        );
                        stats.record_failure();
                        index += 1;
                        continue;
                    }
                }
            }
        };

        process_bulk_image(
            resolved_image,
            &image_title,
            index,
            args,
            stats,
            base_dir,
            bulk_outfile.as_deref(),
        )
        .await;
        index += 1;
    }

    Ok(())
}

async fn process_bulk_image(
    image: ImageDescriptor,
    image_title: &str,
    index: usize,
    args: &Arguments,
    stats: &mut BulkStats,
    base_dir: &Path,
    bulk_outfile: Option<&Path>,
) {
    use log::{debug, trace, warn};

    debug!(
        "Preparing image {}/{}: {image_title}",
        index + 1,
        stats.total_images
    );
    debug!("Resolved {} image", image.format);
    for warning in &image.warnings {
        warn!("{warning}");
    }
    let zoom_levels = image.levels;
    trace!(
        "Zoom levels for image {}: {} levels available",
        index + 1,
        zoom_levels.len()
    );

    let zoom_level = match choose_level(zoom_levels, args) {
        Ok(zoom_level) => zoom_level,
        Err(error) => {
            warn!(
                "Failed to choose a zoom level for image {} ('{image_title}'): {error}",
                index + 1
            );
            stats.record_failure();
            return;
        }
    };
    debug!(
        "Selected zoom level for image {}: {} ({}x{})",
        index + 1,
        zoom_level.title.as_deref().unwrap_or("level"),
        zoom_level.size.map_or(0, |s| s.x),
        zoom_level.size.map_or(0, |s| s.y)
    );

    let level_title = zoom_level
        .title
        .clone()
        .unwrap_or_else(|| image_title.to_owned());
    let indexed_outfile = bulk_outfile.map(|path| generate_bulk_output_name(path, index));
    let save_as = get_outname(
        indexed_outfile.as_deref(),
        Some(&level_title),
        base_dir,
        zoom_level.size,
    );
    if let Err(error) = reserve_output_file(&save_as) {
        let file_name = save_as
            .file_name()
            .map_or_else(|| "unknown".into(), |name| name.to_string_lossy());
        warn!(
            "Failed to prepare output file '{file_name}' for image {} ('{image_title}'): {error}",
            index + 1
        );
        stats.record_failure();
        return;
    }

    info!(
        "Processing image {}/{}: {} -> {}",
        index + 1,
        stats.total_images,
        image_title,
        save_as.file_name().unwrap_or_default().to_string_lossy()
    );
    let tile_buffer = create_tile_buffer(save_as.clone(), args.compression);
    match dezoomify_level(args, zoom_level, tile_buffer).await {
        Ok(()) => {
            info!(
                "Successfully saved image {} to {}",
                index + 1,
                save_as.display()
            );
            stats.record_success();
        }
        Err(ZoomError::PartialDownload {
            successful_tiles,
            total_tiles,
            ..
        }) => {
            warn!(
                "Image {} completed with partial download: {successful_tiles}/{total_tiles} tiles",
                index + 1
            );
            stats.record_partial();
        }
        Err(error) => {
            warn!(
                "Failed to process image {} ('{image_title}'): {error}",
                index + 1
            );
            stats.record_failure();
        }
    }
}

/// Generate a unique output filename for bulk processing
fn generate_bulk_output_name(base_outfile: &Path, index: usize) -> PathBuf {
    let mut result = base_outfile.to_path_buf();

    if let Some(stem) = base_outfile.file_stem() {
        if let Some(extension) = base_outfile.extension() {
            let new_name = format!(
                "{}_{}.{}",
                stem.to_string_lossy(),
                index + 1,
                extension.to_string_lossy()
            );
            result.set_file_name(new_name);
        } else {
            let new_name = format!("{}_{}", stem.to_string_lossy(), index + 1);
            result.set_file_name(new_name);
        }
    } else {
        result.set_file_name(format!("dezoomified_{}.jpg", index + 1));
    }

    result
}

/// Validates the download success based on the final state.
/// Validates that enough tiles were downloaded to proceed
fn validate_download_success(state: &download_state::DownloadState) -> Result<(), ZoomError> {
    if state.is_successful() {
        Ok(())
    } else {
        Err(ZoomError::NoTile)
    }
}

/// Determines final result based on download success rate
fn determine_final_result(
    state: &download_state::DownloadState,
    destination: String,
) -> Result<(), ZoomError> {
    if state.has_partial_failure() {
        Err(ZoomError::PartialDownload {
            successful_tiles: state.successful_tiles,
            total_tiles: state.total_tiles,
            destination,
        })
    } else {
        Ok(())
    }
}

/// Downloads and encodes one zoom level into `tile_buffer`.
///
/// # Errors
///
/// Returns an error if tile downloading or output encoding fails, if no tile succeeds,
/// or if only part of the image can be downloaded.
pub async fn dezoomify_level(
    args: &Arguments,
    level: LevelDescriptor,
    tile_buffer: TileBuffer,
) -> Result<(), ZoomError> {
    debug!("Starting to dezoomify level {:?}", level.id);
    let mut canvas = tile_buffer;
    let state = dezoomify_level_into_buffer(args, level, &mut canvas).await?;
    validate_download_success(&state)?;
    finalize_canvas(&mut canvas).await?;
    let destination = canvas.destination().to_string_lossy().to_string();
    determine_final_result(&state, destination)
}

async fn dezoomify_level_into_buffer(
    args: &Arguments,
    level: LevelDescriptor,
    canvas: &mut TileBuffer,
) -> Result<download_state::DownloadState, ZoomError> {
    let mut coordinator = download_state::TileDownloadCoordinator::new(args)?;
    let mut state = download_state::DownloadState::new();
    let progress = download_state::ProgressManager::new();

    progress.set_computing_urls();

    let level_size = level.size;
    match level.plan {
        LevelPlan::Known(plan) => {
            let mut cursor = plan.cursor();
            while let Some(specs) =
                cursor
                    .take_ready(args.parallelism)
                    .map_err(|error| ZoomError::Dezoomer {
                        message: error.to_string(),
                    })?
            {
                coordinator
                    .download_batch(specs, canvas, &mut state, &progress, level_size)
                    .await?;
            }
        }
        LevelPlan::Adaptive(plan) => {
            let mut program = plan.start();
            loop {
                let Some(specs) =
                    program
                        .take_ready(args.parallelism)
                        .map_err(|error| ZoomError::Dezoomer {
                            message: error.to_string(),
                        })?
                else {
                    break;
                };
                let observations = coordinator
                    .download_batch(
                        specs,
                        canvas,
                        &mut state,
                        &progress,
                        program.image_size().or(level_size),
                    )
                    .await?;
                if !observations.is_empty() {
                    program
                        .submit(&observations)
                        .map_err(|error| ZoomError::Dezoomer {
                            message: error.to_string(),
                        })?;
                }
            }
        }
    }

    progress.finish();
    Ok(state)
}

async fn finalize_canvas(canvas: &mut TileBuffer) -> Result<(), ZoomError> {
    let progress = download_state::ProgressManager::new();
    progress.set_finalizing();
    canvas.finalize().await?;
    progress.finish();
    Ok(())
}

/// Returns the maximal size a tile can have in order to fit in a canvas of the given size
#[must_use]
pub fn max_size_in_rect(position: Vec2d, tile_size: Vec2d, canvas_size: Vec2d) -> Vec2d {
    (position + tile_size).min(canvas_size) - position
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn test_level(
        size: Option<Vec2d>,
        tile_size: Option<Vec2d>,
        overlaps: bool,
    ) -> LevelDescriptor {
        LevelDescriptor {
            id: "test-level".into(),
            title: None,
            size,
            tile_size,
            scale_factor: None,
            has_overlapping_tiles: overlaps,
            plan: LevelPlan::Known(
                dezoomify_core::core::KnownTilePlan::explicit(Vec::new())
                    .expect("empty plan is valid"),
            ),
            provenance: dezoomify_core::core::Provenance::default(),
            warnings: Vec::new(),
        }
    }

    fn test_levels(sizes: &[u32]) -> Vec<LevelDescriptor> {
        sizes
            .iter()
            .map(|&size| test_level(Some(Vec2d::square(size)), None, false))
            .collect()
    }

    fn pyramid_test_levels() -> Vec<LevelDescriptor> {
        [256, 512]
            .into_iter()
            .map(|size| test_level(Some(Vec2d::square(size)), Some(Vec2d::square(256)), false))
            .collect()
    }

    #[test]
    fn deferred_context_reaches_the_resolved_image() {
        let parent = DeferredImage {
            id: "manifest-entry".into(),
            uri: "memory://image/info.json".into(),
            title: Some("Manifest title".into()),
            provenance: dezoomify_core::core::Provenance(vec![
                dezoomify_core::core::ProvenanceStep {
                    id: "manifest-rule".into(),
                    description: "found image service".into(),
                },
            ]),
            warnings: vec!["manifest warning".into()],
        };
        let child = ImageDescriptor {
            id: "iiif-image".into(),
            title: None,
            format: "iiif".into(),
            levels: Vec::new(),
            provenance: dezoomify_core::core::Provenance(vec![
                dezoomify_core::core::ProvenanceStep {
                    id: "iiif".into(),
                    description: "parsed info.json".into(),
                },
            ]),
            warnings: vec!["image warning".into()],
        };

        let catalog =
            inherit_deferred_context(ImageCatalog::new([CatalogEntry::Ready(child)]), &parent);
        let [CatalogEntry::Ready(image)] = catalog.entries() else {
            panic!("resolved child must remain ready")
        };
        assert_eq!(image.title.as_deref(), Some("Manifest title"));
        assert_eq!(image.warnings, ["manifest warning", "image warning"]);
        assert_eq!(
            image
                .provenance
                .0
                .iter()
                .map(|step| step.id.as_str())
                .collect::<Vec<_>>(),
            ["manifest-rule", "iiif"]
        );
    }

    #[test]
    fn test_parse_level_index() {
        assert_eq!(parse_level_index("0", 5), Some(0));
        assert_eq!(parse_level_index("4", 5), Some(4));
        assert_eq!(parse_level_index("5", 5), None); // Out of bounds
        assert_eq!(parse_level_index("abc", 5), None); // Invalid number
        assert_eq!(parse_level_index("", 5), None); // Empty string
        assert_eq!(parse_level_index("2", 1), None); // Index too high
    }

    #[test]
    fn test_resolve_level_index() {
        assert_eq!(resolve_level_index(2, 5), 2); // Within bounds
        assert_eq!(resolve_level_index(0, 5), 0); // First index
        assert_eq!(resolve_level_index(4, 5), 4); // Last valid index
        assert_eq!(resolve_level_index(10, 5), 4); // Out of bounds, use last
        assert_eq!(resolve_level_index(100, 3), 2); // Way out of bounds
    }

    #[test]
    fn choose_level_indexes_levels_from_smallest_to_largest() {
        let mut args = Arguments::default();
        args.zoom_level = Some(0);
        let selected = choose_level(test_levels(&[100, 200, 400]), &args).unwrap();
        assert_eq!(selected.size, Some(Vec2d::square(100)));

        args.zoom_level = Some(1);
        let selected = choose_level(test_levels(&[100, 200, 400]), &args).unwrap();
        assert_eq!(selected.size, Some(Vec2d::square(200)));

        args.zoom_level = Some(10);
        let selected = choose_level(test_levels(&[100, 200, 400]), &args).unwrap();
        assert_eq!(selected.size, Some(Vec2d::square(400)));
    }

    #[test]
    fn test_resolve_image_index() {
        assert_eq!(resolve_image_index(1, 3), 1); // Within bounds
        assert_eq!(resolve_image_index(0, 3), 0); // First index
        assert_eq!(resolve_image_index(2, 3), 2); // Last valid index
        assert_eq!(resolve_image_index(5, 3), 2); // Out of bounds, use last
        assert_eq!(resolve_image_index(100, 1), 0); // Way out of bounds
    }

    #[test]
    fn test_max_size_in_rect() {
        // Tile fits completely within canvas
        assert_eq!(
            max_size_in_rect(
                Vec2d { x: 10, y: 10 },
                Vec2d { x: 50, y: 50 },
                Vec2d { x: 100, y: 100 }
            ),
            Vec2d { x: 50, y: 50 }
        );

        // Tile extends beyond canvas horizontally
        assert_eq!(
            max_size_in_rect(
                Vec2d { x: 80, y: 10 },
                Vec2d { x: 50, y: 50 },
                Vec2d { x: 100, y: 100 }
            ),
            Vec2d { x: 20, y: 50 }
        );

        // Tile extends beyond canvas vertically
        assert_eq!(
            max_size_in_rect(
                Vec2d { x: 10, y: 80 },
                Vec2d { x: 50, y: 50 },
                Vec2d { x: 100, y: 100 }
            ),
            Vec2d { x: 50, y: 20 }
        );

        // Tile extends beyond canvas in both dimensions
        assert_eq!(
            max_size_in_rect(
                Vec2d { x: 90, y: 90 },
                Vec2d { x: 50, y: 50 },
                Vec2d { x: 100, y: 100 }
            ),
            Vec2d { x: 10, y: 10 }
        );

        // Tile at canvas edge
        assert_eq!(
            max_size_in_rect(
                Vec2d { x: 0, y: 0 },
                Vec2d { x: 100, y: 100 },
                Vec2d { x: 100, y: 100 }
            ),
            Vec2d { x: 100, y: 100 }
        );
    }

    #[test]
    fn source_level_scale_factor_uses_relative_hints() {
        assert_eq!(
            source_level_scale_factor_from_hint(
                Vec2d { x: 5156, y: 3816 },
                Vec2d { x: 2578, y: 1908 },
                Some(2),
                1,
            ),
            2
        );
        assert_eq!(
            source_level_scale_factor_from_hint(
                Vec2d { x: 515, y: 381 },
                Vec2d { x: 515, y: 381 },
                Some(10),
                10,
            ),
            1
        );
    }

    #[test]
    fn source_pyramid_requires_compatible_output_and_levels() {
        let mut args = Arguments::default();
        for extension in ["iiif", "tif", "tiff", "zif"] {
            let path = PathBuf::from(format!("output.{extension}"));
            assert!(can_dezoomify_source_pyramid(
                &path,
                &args,
                &pyramid_test_levels()
            ));
        }
        assert!(!can_dezoomify_source_pyramid(
            Path::new("output.png"),
            &args,
            &pyramid_test_levels()
        ));

        args.largest = true;
        assert!(!can_dezoomify_source_pyramid(
            Path::new("output.tiff"),
            &args,
            &pyramid_test_levels()
        ));
        args.largest = false;
        args.zoom_level = Some(0);
        assert!(!can_dezoomify_source_pyramid(
            Path::new("output.tiff"),
            &args,
            &pyramid_test_levels()
        ));

        let missing_size = vec![test_level(None, Some(Vec2d::square(256)), false)];
        assert!(!can_dezoomify_source_pyramid(
            Path::new("output.tiff"),
            &Arguments::default(),
            &missing_size
        ));

        let missing_tile_size = vec![test_level(Some(Vec2d::square(512)), None, false)];
        assert!(!can_dezoomify_source_pyramid(
            Path::new("output.tiff"),
            &Arguments::default(),
            &missing_tile_size
        ));

        let overlapping = vec![test_level(
            Some(Vec2d::square(512)),
            Some(Vec2d::square(256)),
            true,
        )];
        assert!(!can_dezoomify_source_pyramid(
            Path::new("output.tiff"),
            &Arguments::default(),
            &overlapping
        ));
    }

    #[test]
    fn source_level_scale_factor_falls_back_for_unusable_hints() {
        assert_eq!(
            source_level_scale_factor_from_hint(
                Vec2d { x: 5156, y: 3816 },
                Vec2d { x: 2578, y: 1908 },
                None,
                1,
            ),
            2
        );
        assert_eq!(
            source_level_scale_factor_from_hint(
                Vec2d { x: 5156, y: 3816 },
                Vec2d { x: 2578, y: 1908 },
                Some(3),
                2,
            ),
            2
        );
    }

    #[test]
    fn test_validate_download_success() {
        let mut successful_state = download_state::DownloadState::new();
        successful_state.record_output_success();
        assert!(validate_download_success(&successful_state).is_ok());

        let failed_state = download_state::DownloadState::new();
        assert!(validate_download_success(&failed_state).is_err());
    }

    #[test]
    fn test_determine_final_result() {
        let destination = "test.jpg".to_string();

        // Complete success - no partial failure
        let mut success_state = download_state::DownloadState::new();
        success_state.add_batch(10);
        for _ in 0..10 {
            success_state.record_output_success();
        }
        assert!(determine_final_result(&success_state, destination.clone()).is_ok());

        // Partial failure
        let mut partial_state = download_state::DownloadState::new();
        partial_state.add_batch(10);
        for _ in 0..8 {
            partial_state.record_output_success();
        }
        let result = determine_final_result(&partial_state, destination.clone());
        assert!(result.is_err());
        if let Err(ZoomError::PartialDownload {
            successful_tiles,
            total_tiles,
            ..
        }) = result
        {
            assert_eq!(successful_tiles, 8);
            assert_eq!(total_tiles, 10);
        } else {
            panic!("Expected PartialDownload error");
        }
    }

    #[test]
    fn test_find_level_with_size() {
        // Test the size-selection predicate directly with a simple set of hints.
        let sizes = [
            Some(Vec2d { x: 100, y: 100 }),
            Some(Vec2d { x: 200, y: 200 }),
            None,
            Some(Vec2d { x: 300, y: 300 }),
        ];

        let target_size = Vec2d { x: 200, y: 200 };
        let position = sizes.iter().position(|&s| s == Some(target_size));
        assert_eq!(position, Some(1));

        let target_size_not_found = Vec2d { x: 400, y: 400 };
        let position = sizes.iter().position(|&s| s == Some(target_size_not_found));
        assert_eq!(position, None);
    }

    #[test]
    fn test_generate_bulk_output_name() {
        use std::path::Path;

        // Test with extension
        let base = Path::new("output.jpg");
        assert_eq!(
            generate_bulk_output_name(base, 0),
            Path::new("output_1.jpg")
        );
        assert_eq!(
            generate_bulk_output_name(base, 9),
            Path::new("output_10.jpg")
        );

        // Test without extension
        let base = Path::new("output");
        assert_eq!(generate_bulk_output_name(base, 0), Path::new("output_1"));
        assert_eq!(generate_bulk_output_name(base, 4), Path::new("output_5"));

        // Test with complex path
        let base = Path::new("/path/to/my_file.png");
        assert_eq!(
            generate_bulk_output_name(base, 2),
            Path::new("/path/to/my_file_3.png")
        );

        // Test with no stem (edge case)
        let base = Path::new(".hidden");
        assert_eq!(generate_bulk_output_name(base, 0), Path::new(".hidden_1"));
    }

    #[test]
    fn test_bulk_stats() {
        let mut stats = BulkStats::new();

        // Test initial state
        assert_eq!(stats.total_images, 0);
        assert_eq!(stats.successful_images, 0);
        assert_eq!(stats.failed_images, 0);
        assert_eq!(stats.partial_downloads, 0);

        // Test setting total
        stats.set_total(10);
        assert_eq!(stats.total_images, 10);

        // Test recording different types of results
        stats.record_success();
        stats.record_success();
        stats.record_partial();
        stats.record_failure();
        stats.record_failure();
        stats.record_failure();

        assert_eq!(stats.successful_images, 2);
        assert_eq!(stats.partial_downloads, 1);
        assert_eq!(stats.failed_images, 3);
        assert_eq!(stats.total_images, 10); // Should remain unchanged
    }

    #[test]
    fn test_generate_bulk_output_name_edge_cases() {
        use std::path::Path;

        // Test with multiple dots
        let base = Path::new("file.name.with.dots.jpg");
        assert_eq!(
            generate_bulk_output_name(base, 0),
            Path::new("file.name.with.dots_1.jpg")
        );

        // Test with extension only
        let base = Path::new(".jpg");
        assert_eq!(generate_bulk_output_name(base, 0), Path::new(".jpg_1"));

        // Test large index
        let base = Path::new("test.png");
        assert_eq!(
            generate_bulk_output_name(base, 999),
            Path::new("test_1000.png")
        );

        // Test with Unicode characters
        let base = Path::new("测试文件.jpg");
        assert_eq!(
            generate_bulk_output_name(base, 0),
            Path::new("测试文件_1.jpg")
        );
    }

    #[test]
    fn test_bulk_mode_outfile_prefers_explicit_outfile() {
        let args = Arguments::parse_from([
            "dezoomify-rs",
            "--bulk",
            "urls.txt",
            "from_positional.jpg",
            "explicit.jpg",
        ]);
        assert_eq!(args.bulk_output_file(), Some(PathBuf::from("explicit.jpg")));
    }

    #[test]
    fn test_bulk_mode_outfile_does_not_use_input_uri() {
        let args = Arguments::parse_from(["dezoomify-rs", "--bulk", "urls.txt", "fallback.jpg"]);
        assert_eq!(args.bulk_output_file(), None);
    }

    #[test]
    fn test_bulk_mode_outfile_option_overrides_positionals() {
        let args = Arguments::parse_from([
            "dezoomify-rs",
            "--bulk",
            "urls.txt",
            "positional-input.jpg",
            "--outfile",
            "from-option.jpg",
        ]);
        assert_eq!(
            args.bulk_output_file(),
            Some(PathBuf::from("from-option.jpg"))
        );
    }
}
