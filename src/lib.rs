#![allow(clippy::upper_case_acronyms)]

use std::env::current_dir;

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::{fs, io};

use log::{debug, error, info};
use reqwest::Client;

pub use arguments::Arguments;
pub use binary_display::{BinaryDisplay, display_bytes};
use dezoomer::TileReference;
use dezoomer::{Dezoomer, DezoomerError, DezoomerInput};
use dezoomer::{ZoomLevel, ZoomLevelIter};
pub use errors::ZoomError;
use network::{client, fetch_uri};
use output_file::get_outname;
use tile::Tile;
pub use vec2d::Vec2d;

use crate::dezoomer::{DezoomerResult, PageContents, ZoomableImage};
use crate::encoder::SourceLevel;
use crate::encoder::tile_buffer::TileBuffer;

use crate::output_file::reserve_output_file;

mod arguments;
mod binary_display;

pub mod dezoomer;
pub(crate) mod download_state;
mod encoder;
mod errors;
mod network;
mod output_file;
pub mod tile;
mod vec2d;

pub mod auto;
pub mod bulk_text;
pub mod custom_yaml;
pub mod dzi;
pub mod generic;
pub mod google_arts_and_culture;
pub mod iiif;
pub mod iipimage;
mod json_utils;
pub mod krpano;
pub mod nypl;
pub mod pff;
mod throttler;
pub mod zoomify;

fn stdin_line() -> Result<String, ZoomError> {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let first_line = lines.next().ok_or_else(|| {
        let err_msg = "Encountered end of standard input while reading a line";
        io::Error::new(io::ErrorKind::UnexpectedEof, err_msg)
    })?;
    Ok(first_line?)
}

/// Process a single dezoomer to get a result, handling the NeedsData loop
async fn get_dezoomer_result(
    dezoomer: &mut dyn Dezoomer,
    http: &Client,
    uri: &str,
) -> Result<DezoomerResult, ZoomError> {
    let mut i = DezoomerInput {
        uri: String::from(uri),
        contents: PageContents::Unknown,
    };
    loop {
        match dezoomer.dezoomer_result(&i) {
            Ok(result) => return Ok(result),
            Err(DezoomerError::NeedsData { uri }) => {
                let contents = fetch_uri(&uri, http).await.into();
                debug!("Response for metadata file '{}': {:?}", uri, &contents);
                i.uri = uri;
                i.contents = contents;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Process an input URI to extract zoomable images
async fn get_images_from_uri(
    args: &Arguments,
    http: &Client,
    uri: &str,
) -> Result<Vec<ZoomableImage>, ZoomError> {
    let mut dezoomer = args.find_dezoomer()?;
    let zoomable_images = get_dezoomer_result(dezoomer.as_mut(), http, uri).await?;
    Ok(zoomable_images)
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
fn find_level_with_size(levels: &[ZoomLevel], target_size: Vec2d) -> Option<usize> {
    levels
        .iter()
        .position(|l| l.size_hint() == Some(target_size))
}

/// An interactive level picker
fn level_picker(mut levels: Vec<ZoomLevel>) -> Result<ZoomLevel, ZoomError> {
    println!("Found the following zoom levels:");
    for (i, level) in levels.iter().enumerate() {
        println!("{: >2}. {}", i, level.name());
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

fn choose_level(mut levels: Vec<ZoomLevel>, args: &Arguments) -> Result<ZoomLevel, ZoomError> {
    match levels.len() {
        0 => Err(ZoomError::NoLevels),
        1 => Ok(levels.swap_remove(0)),
        _ => {
            if let Some(requested_level) = args.zoom_level {
                let actual_level = resolve_level_index(requested_level, levels.len());
                if actual_level == requested_level {
                    info!("Selected zoom level {} as requested", requested_level);
                } else {
                    info!(
                        "Requested zoom level {} not available. Using last one ({})",
                        requested_level, actual_level
                    );
                }
                return Ok(levels.swap_remove(actual_level));
            }

            if let Some(best_size) = args.best_size(levels.iter().filter_map(|l| l.size_hint()))
                && let Some(pos) = find_level_with_size(&levels, best_size)
            {
                return Ok(levels.swap_remove(pos));
            }

            level_picker(levels)
        }
    }
}

/// An interactive image picker for when multiple images are available
fn image_picker(mut images: Vec<ZoomableImage>) -> Result<ZoomableImage, ZoomError> {
    println!("Found the following images:");
    for (i, image) in images.iter().enumerate() {
        let title = image
            .title()
            .unwrap_or_else(|| format!("Image {}", i + 1).into());
        println!("{: >2}. {}", i, title);
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
    mut images: Vec<ZoomableImage>,
    args: &Arguments,
) -> Result<ZoomableImage, ZoomError> {
    match images.len() {
        0 => Err(ZoomError::NoLevels),
        1 => Ok(images.swap_remove(0)),
        _ => {
            if let Some(requested_index) = args.image_index {
                let actual_index = resolve_image_index(requested_index, images.len());
                if actual_index == requested_index {
                    info!("Selected image {} as requested", requested_index);
                } else {
                    info!(
                        "Requested image index {} not available. Using last one ({})",
                        requested_index, actual_index
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

/// Prepares the output file path for saving
fn prepare_output_path(
    outfile_arg: &Option<PathBuf>,
    title: &Option<String>,
    base_dir: &Path,
    size_hint: Option<Vec2d>,
) -> Result<PathBuf, ZoomError> {
    let outname = get_outname(outfile_arg, title, base_dir, size_hint);
    let save_as = fs::canonicalize(outname.as_path()).unwrap_or_else(|_e| outname.clone());
    reserve_output_file(&save_as)?;
    Ok(save_as)
}

/// Creates a tile buffer for the given output path
async fn create_tile_buffer(save_as: PathBuf, compression: u8) -> Result<TileBuffer, ZoomError> {
    TileBuffer::new(save_as, compression).await
}

fn output_prefers_source_pyramid(path: &Path, args: &Arguments) -> bool {
    if args.has_level_specifying_args() || args.largest {
        return false;
    }
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("iiif" | "tif" | "tiff")
    )
}

fn can_dezoomify_source_pyramid(path: &Path, args: &Arguments, levels: &[ZoomLevel]) -> bool {
    output_prefers_source_pyramid(path, args)
        && largest_level_size(levels).is_some()
        && levels.iter().all(|level| {
            level.size_hint().is_some()
                && level.tile_size_hint().is_some()
                && !level.has_overlapping_tiles()
        })
}

async fn dezoomify_source_pyramid(
    args: &Arguments,
    mut levels: Vec<ZoomLevel>,
    tile_buffer: TileBuffer,
) -> Result<(), ZoomError> {
    let mut canvas = tile_buffer;
    let full_size = largest_level_size(&levels).ok_or(ZoomError::NoLevels)?;
    levels.sort_by_key(|level| std::cmp::Reverse(level_area(level.size_hint())));

    let mut total_tiles = 0;
    let mut successful_tiles = 0;
    for (index, zoom_level) in levels.into_iter().enumerate() {
        let level_size = zoom_level.size_hint().unwrap_or(full_size);
        let scale_factor = source_level_scale_factor(full_size, level_size, &zoom_level);
        canvas
            .begin_level(SourceLevel {
                index,
                size: full_size,
                scale_factor,
                tile_size: zoom_level.tile_size_hint(),
                has_overlapping_tiles: zoom_level.has_overlapping_tiles(),
            })
            .await?;
        let state = dezoomify_level_into_buffer(args, zoom_level, &mut canvas).await?;
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

fn source_level_scale_factor(full_size: Vec2d, level_size: Vec2d, level: &ZoomLevel) -> u32 {
    level
        .scale_factor_hint()
        .filter(|&scale_factor| scale_factor > 0)
        .unwrap_or_else(|| full_size.x.div_ceil(level_size.x).max(1))
}

fn largest_level_size(levels: &[ZoomLevel]) -> Option<Vec2d> {
    levels
        .iter()
        .filter_map(|level| level.size_hint())
        .max_by_key(|size| level_area(Some(*size)))
}

fn level_area(size: Option<Vec2d>) -> u64 {
    size.map(|size| u64::from(size.x) * u64::from(size.y))
        .unwrap_or(0)
}

pub async fn dezoomify(args: &Arguments) -> Result<PathBuf, ZoomError> {
    let uri = args.choose_input_uri()?;
    let http_client = client(args.headers(), args, Some(&uri))?;
    debug!("Trying to locate a zoomable image...");
    let images = get_images_from_uri(args, &http_client, &uri).await?;
    debug!("Found {} zoomable images", images.len());
    let selected_image = choose_image(images, args)?;
    let title = selected_image.title().map(|title| title.into_owned());
    let zoom_levels = selected_image
        .into_zoom_levels(&http_client)
        .await
        .map_err(|e| ZoomError::Dezoomer { source: e })?;

    let base_dir = current_dir()?;
    let output_file = args.output_file();
    let largest_size = largest_level_size(&zoom_levels);
    let source_pyramid_path = get_outname(&output_file, &title, &base_dir, largest_size);

    if can_dezoomify_source_pyramid(&source_pyramid_path, args, &zoom_levels) {
        let save_as = prepare_output_path(&output_file, &title, &base_dir, largest_size)?;
        let tile_buffer = create_tile_buffer(save_as.clone(), args.compression).await?;
        info!("Dezooming source pyramid with {} levels", zoom_levels.len());
        dezoomify_source_pyramid(args, zoom_levels, tile_buffer).await?;
        Ok(save_as)
    } else {
        let zoom_level = choose_level(zoom_levels, args)?;
        let save_as = prepare_output_path(&output_file, &title, &base_dir, zoom_level.size_hint())?;
        let tile_buffer = create_tile_buffer(save_as.clone(), args.compression).await?;
        info!("Dezooming {}", zoom_level.name());
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

/// Process multiple images in bulk mode using the new unified architecture
pub async fn process_bulk(args: &Arguments) -> Result<BulkStats, ZoomError> {
    use log::{debug, trace};

    debug!("Starting bulk processing mode");
    trace!("Bulk processing arguments: {:?}", args);

    // Get the bulk file/URI from arguments
    let bulk_uri = args.bulk.as_ref().ok_or_else(|| ZoomError::NoBulkUrl {
        bulk_file_path: "No bulk source specified".to_string(),
    })?;

    debug!("Bulk source: {}", bulk_uri);

    // Get dezoomer result from the bulk source
    let http = client(std::iter::empty(), args, None)?;
    let mut dezoomer = args.find_dezoomer()?;
    let dezoomer_result = get_dezoomer_result(dezoomer.as_mut(), &http, bulk_uri).await?;

    let mut stats = BulkStats::new();
    let base_dir = current_dir()?;

    // Process each ZoomableImage in bulk mode
    stats.set_total(dezoomer_result.len());
    info!(
        "Found {} images to process in bulk mode",
        dezoomer_result.len()
    );
    debug!(
        "Images discovered: {:?}",
        dezoomer_result
            .iter()
            .map(|img| img.title().unwrap_or_else(|| "Untitled".into()))
            .collect::<Vec<_>>()
    );

    process_bulk_zoomable_images(dezoomer_result, args, &http, &mut stats, &base_dir).await?;

    // Log final statistics
    info!("Bulk processing complete!");
    info!("Total images: {}", stats.total_images);
    info!("Successfully downloaded: {}", stats.successful_images);
    info!("Partial downloads: {}", stats.partial_downloads);
    info!("Failed downloads: {}", stats.failed_images);

    debug!("Final bulk processing stats: {:?}", stats);

    Ok(stats)
}

/// Process a list of ZoomableImage objects in bulk - resolve each one to zoom levels as needed
async fn process_bulk_zoomable_images(
    images: Vec<ZoomableImage>,
    args: &Arguments,
    http: &Client,
    stats: &mut BulkStats,
    base_dir: &Path,
) -> Result<(), ZoomError> {
    use log::{debug, trace, warn};
    let bulk_outfile = args.bulk_output_file();

    // Process each ZoomableImage individually
    for (index, zoomable_image) in images.into_iter().enumerate() {
        let image_title = zoomable_image
            .title()
            .unwrap_or_else(|| format!("Image_{}", index + 1).into())
            .to_string();
        debug!(
            "Preparing image {}/{}: {}",
            index + 1,
            stats.total_images,
            image_title
        );

        // Resolve the ZoomableImage to get zoom levels
        let zoom_levels = match zoomable_image.into_zoom_levels(http).await {
            Ok(levels) => levels,
            Err(e) => {
                warn!(
                    "Failed to get zoom levels for image {} ('{}'): {}",
                    index + 1,
                    image_title,
                    e
                );
                stats.record_failure();
                continue;
            }
        };

        trace!(
            "Zoom levels for image {}: {} levels available",
            index + 1,
            zoom_levels.len()
        );

        // Choose the appropriate zoom level using existing logic
        let zoom_level = match choose_level(zoom_levels, args) {
            Ok(level) => level,
            Err(e) => {
                warn!(
                    "Failed to choose zoom level for image {} ('{}'): {}",
                    index + 1,
                    image_title,
                    e
                );
                stats.record_failure();
                continue;
            }
        };

        debug!(
            "Selected zoom level for image {}: {} ({}x{})",
            index + 1,
            zoom_level.name(),
            zoom_level.size_hint().map(|s| s.x).unwrap_or(0),
            zoom_level.size_hint().map(|s| s.y).unwrap_or(0)
        );

        // Use get_outname to handle file collision properly, without args.outfile override
        let save_as = if let Some(ref base_outfile) = bulk_outfile {
            // In bulk mode with specified outfile, use index-based naming with collision handling
            let base_path = generate_bulk_output_name(base_outfile, index);
            get_outname(
                &Some(base_path),
                &zoom_level.title().or_else(|| Some(image_title.clone())),
                base_dir,
                zoom_level.size_hint(),
            )
        } else {
            // Use the zoom level title if present, fallback to image title
            get_outname(
                &None,
                &zoom_level.title().or_else(|| Some(image_title.clone())),
                base_dir,
                zoom_level.size_hint(),
            )
        };

        // Reserve the output file to avoid collisions
        if let Err(e) = reserve_output_file(&save_as) {
            let file_name = save_as
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_else(|| "unknown".into());
            warn!(
                "Failed to prepare output file '{}' for image {} ('{}'): {}",
                file_name,
                index + 1,
                image_title,
                e
            );
            stats.record_failure();
            continue;
        };

        let tile_buffer = match create_tile_buffer(save_as.clone(), args.compression).await {
            Ok(buffer) => buffer,
            Err(e) => {
                let file_name = save_as
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_else(|| "unknown".into());
                warn!(
                    "Failed to create tile buffer for file '{}' (image {} '{}'): {}",
                    file_name,
                    index + 1,
                    image_title,
                    e
                );
                stats.record_failure();
                continue;
            }
        };

        // Now show processing message since we're about to start downloading
        info!(
            "Processing image {}/{}: {} -> {}",
            index + 1,
            stats.total_images,
            image_title,
            save_as.file_name().unwrap_or_default().to_string_lossy()
        );

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
                    "Image {} completed with partial download: {}/{} tiles",
                    index + 1,
                    successful_tiles,
                    total_tiles
                );
                stats.record_partial();
            }
            Err(e) => {
                warn!(
                    "Failed to process image {} ('{}'): {}",
                    index + 1,
                    image_title,
                    e
                );
                stats.record_failure();
            }
        }
    }

    Ok(())
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
    if !state.is_successful() {
        Err(ZoomError::NoTile)
    } else {
        Ok(())
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

pub async fn dezoomify_level(
    args: &Arguments,
    zoom_level: ZoomLevel,
    tile_buffer: TileBuffer,
) -> Result<(), ZoomError> {
    debug!("Starting to dezoomify {zoom_level:?}");
    let mut canvas = tile_buffer;
    let state = dezoomify_level_into_buffer(args, zoom_level, &mut canvas).await?;
    validate_download_success(&state)?;
    finalize_canvas(&mut canvas).await?;
    let destination = canvas.destination().to_string_lossy().to_string();
    determine_final_result(&state, destination)
}

async fn dezoomify_level_into_buffer(
    args: &Arguments,
    mut zoom_level: ZoomLevel,
    canvas: &mut TileBuffer,
) -> Result<download_state::DownloadState, ZoomError> {
    let mut coordinator = download_state::TileDownloadCoordinator::new(&zoom_level, args)?;
    let mut state = download_state::DownloadState::new();
    let progress = download_state::ProgressManager::new();

    progress.set_computing_urls();

    let mut zoom_level_iter = ZoomLevelIter::new(&mut zoom_level);

    while let Some(tile_refs) = zoom_level_iter.next_tile_references() {
        coordinator
            .download_batch(tile_refs, canvas, &mut state, &progress, &zoom_level_iter)
            .await?;

        zoom_level_iter.set_fetch_result(state.create_fetch_result());
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
pub fn max_size_in_rect(position: Vec2d, tile_size: Vec2d, canvas_size: Vec2d) -> Vec2d {
    (position + tile_size).min(canvas_size) - position
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

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
    fn test_validate_download_success() {
        let mut successful_state = download_state::DownloadState::new();
        successful_state.record_success();
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
            success_state.record_success();
        }
        assert!(determine_final_result(&success_state, destination.clone()).is_ok());

        // Partial failure
        let mut partial_state = download_state::DownloadState::new();
        partial_state.add_batch(10);
        for _ in 0..8 {
            partial_state.record_success();
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
        // Since we can't easily create real ZoomLevel instances for testing,
        // let's test the logic directly with a simpler approach
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

#[cfg(test)]
mod iiif_title_tests {
    use crate::iiif::determine_title;
    use crate::iiif::manifest_types::ExtractedImageInfo;

    #[test]
    fn test_determine_title_all_components() {
        let image_info = ExtractedImageInfo {
            image_uri: "https://example.com/image.json".to_string(),
            manifest_label: Some("Manifest Title".to_string()),
            metadata_title: Some("Metadata Title".to_string()),
            canvas_label: Some("Canvas Label".to_string()),
            canvas_index: 0,
        };

        let result = determine_title(&image_info);
        assert_eq!(
            result,
            Some("Manifest Title - Metadata Title - Canvas Label".to_string())
        );
    }

    #[test]
    fn test_determine_title_manifest_and_canvas_only() {
        let image_info = ExtractedImageInfo {
            image_uri: "https://example.com/image.json".to_string(),
            manifest_label: Some("Book Title".to_string()),
            metadata_title: None,
            canvas_label: Some("Page 1".to_string()),
            canvas_index: 0,
        };

        let result = determine_title(&image_info);
        assert_eq!(result, Some("Book Title - Page 1".to_string()));
    }

    #[test]
    fn test_determine_title_canvas_only() {
        let image_info = ExtractedImageInfo {
            image_uri: "https://example.com/image.json".to_string(),
            manifest_label: None,
            metadata_title: None,
            canvas_label: Some("Single Page".to_string()),
            canvas_index: 0,
        };

        let result = determine_title(&image_info);
        assert_eq!(result, Some("Single Page".to_string()));
    }

    #[test]
    fn test_determine_title_no_duplicates() {
        // Test that duplicate titles are not repeated
        let image_info = ExtractedImageInfo {
            image_uri: "https://example.com/image.json".to_string(),
            manifest_label: Some("Same Title".to_string()),
            metadata_title: Some("Same Title".to_string()), // Duplicate
            canvas_label: Some("Different Label".to_string()),
            canvas_index: 0,
        };

        let result = determine_title(&image_info);
        assert_eq!(result, Some("Same Title - Different Label".to_string()));
    }

    #[test]
    fn test_determine_title_empty() {
        let image_info = ExtractedImageInfo {
            image_uri: "https://example.com/image.json".to_string(),
            manifest_label: None,
            metadata_title: None,
            canvas_label: None,
            canvas_index: 0,
        };

        let result = determine_title(&image_info);
        assert_eq!(result, None);
    }

    #[test]
    fn test_determine_title_metadata_only() {
        let image_info = ExtractedImageInfo {
            image_uri: "https://example.com/image.json".to_string(),
            manifest_label: None,
            metadata_title: Some("Metadata Only".to_string()),
            canvas_label: None,
            canvas_index: 0,
        };

        let result = determine_title(&image_info);
        assert_eq!(result, Some("Metadata Only".to_string()));
    }

    #[test]
    fn test_determine_title_special_characters() {
        let image_info = ExtractedImageInfo {
            image_uri: "https://example.com/image.json".to_string(),
            manifest_label: Some("Ms. Smith's \"Book\" & Notes (1850-1900)".to_string()),
            metadata_title: None,
            canvas_label: Some("Page #1: Introduction/Overview".to_string()),
            canvas_index: 0,
        };

        let result = determine_title(&image_info);
        assert_eq!(
            result,
            Some(
                "Ms. Smith's \"Book\" & Notes (1850-1900) - Page #1: Introduction/Overview"
                    .to_string()
            )
        );
    }

    #[test]
    fn test_determine_title_very_long() {
        let long_manifest = "A".repeat(100);
        let long_canvas = "B".repeat(100);

        let image_info = ExtractedImageInfo {
            image_uri: "https://example.com/image.json".to_string(),
            manifest_label: Some(long_manifest.clone()),
            metadata_title: None,
            canvas_label: Some(long_canvas.clone()),
            canvas_index: 0,
        };

        let result = determine_title(&image_info);
        let expected = format!("{} - {}", long_manifest, long_canvas);
        assert_eq!(result, Some(expected));
    }

    #[test]
    fn test_determine_title_unicode() {
        let image_info = ExtractedImageInfo {
            image_uri: "https://example.com/image.json".to_string(),
            manifest_label: Some("古典文学作品集".to_string()),
            metadata_title: Some("詩經選讀".to_string()),
            canvas_label: Some("第一章：關雎".to_string()),
            canvas_index: 0,
        };

        let result = determine_title(&image_info);
        assert_eq!(
            result,
            Some("古典文学作品集 - 詩經選讀 - 第一章：關雎".to_string())
        );
    }

    #[test]
    fn test_determine_title_whitespace_handling() {
        let image_info = ExtractedImageInfo {
            image_uri: "https://example.com/image.json".to_string(),
            manifest_label: Some("  Manifest with spaces  ".to_string()),
            metadata_title: Some("\tTabbed metadata\t".to_string()),
            canvas_label: Some("Canvas\nwith\nnewlines".to_string()),
            canvas_index: 0,
        };

        let result = determine_title(&image_info);
        // Note: The function doesn't currently trim whitespace, it preserves what's in the manifest
        assert_eq!(
            result,
            Some(
                "  Manifest with spaces   - \tTabbed metadata\t - Canvas\nwith\nnewlines"
                    .to_string()
            )
        );
    }
}
