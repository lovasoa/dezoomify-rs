//! Download bookkeeping, progress bars, and coordination of concurrent tile downloads.

use crate::arguments::Arguments;
use crate::encoder::tile_buffer::TileBuffer;
use crate::errors::{TileDownloadError, ZoomError};
use crate::max_size_in_rect;
use crate::network::{
    TileDownloader, client as network_client, client_cache_headers, user_header_names,
};
use crate::throttler::Throttler;
use crate::tile::{EncodedTile, Tile, load_encoded_tile, load_tile_with_metadata};
use dezoomify_core::Vec2d;
use dezoomify_core::core::{ObservationResult, TileRole, TileSourceError, TileSpec};

use futures::stream::StreamExt;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use log::debug;
use std::default::Default;

// --- DownloadState ---
#[derive(Debug, Default)]
pub(crate) struct DownloadState {
    pub(crate) total_tiles: u64,
    pub(crate) successful_tiles: u64,
    any_successful_tiles: u64,
    tile_size: Option<Vec2d>,
}

impl DownloadState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn set_total_tiles(&mut self, count: u64) {
        self.total_tiles = count;
    }

    pub(crate) fn record_output_success(&mut self) {
        self.successful_tiles += 1;
        self.any_successful_tiles += 1;
    }

    fn record_probe_output_success(&mut self) {
        self.total_tiles += 1;
        self.successful_tiles += 1;
        self.any_successful_tiles += 1;
    }

    fn set_tile_size(&mut self, size: Vec2d) {
        self.tile_size = Some(size);
    }

    pub(crate) fn is_successful(&self) -> bool {
        self.any_successful_tiles > 0
    }

    pub(crate) fn has_partial_failure(&self) -> bool {
        self.successful_tiles < self.total_tiles
    }
}

// --- ProgressManager ---
#[derive(Debug)]
pub(crate) struct ProgressManager {
    progress: ProgressBar,
}

impl ProgressManager {
    pub(crate) fn new() -> Self {
        let progress = progress_bar(10); // Default initial size, will be updated
        if !log::log_enabled!(log::Level::Info) {
            progress.set_draw_target(ProgressDrawTarget::hidden());
        }
        Self { progress }
    }

    pub(crate) fn set_total_tiles(&self, total: u64) {
        self.progress.set_length(total);
    }

    pub(crate) fn set_resolved_tiles(&self, total: u64, completed: u64) {
        self.progress.set_length(total);
        self.progress.set_position(completed);
    }

    pub(crate) fn set_computing_urls(&self) {
        self.progress
            .set_message("Computing the URLs of the image tiles...");
    }

    pub(crate) fn set_requesting_tiles(&self) {
        self.progress.set_message("Requesting the tiles...");
    }

    pub(crate) fn set_finalizing(&self) {
        self.progress
            .set_message("Downloaded all tiles. Finalizing the image file.");
    }

    pub(crate) fn increment(&self) {
        self.progress.inc(1);
    }

    pub(crate) fn update_for_tile(&self, tile: Option<&Tile>, success: bool) {
        if success {
            if let Some(tile) = tile {
                self.progress
                    .set_message(format!("Loaded tile at {}", tile.position()));
            }
        } else {
            self.progress
                .set_message("Failed to load tile, using empty replacement");
        }
    }

    pub(crate) fn update_for_encoded_tile(&self, tile: Option<&EncodedTile>, success: bool) {
        if success {
            if let Some(tile) = tile {
                self.progress
                    .set_message(format!("Loaded encoded tile at {}", tile.position));
            }
        } else {
            self.progress.set_message("Failed to load encoded tile");
        }
    }

    pub(crate) fn finish(&self) {
        self.progress.finish_with_message("Finished tile download");
    }
}

// Helper function, private to this module
fn progress_bar(n: usize) -> ProgressBar {
    let progress = ProgressBar::new(n as u64);
    progress.set_style(
        ProgressStyle::default_bar()
            .template("[ETA:{eta}] {bar:40.cyan/blue} {pos:>4}/{len:4} {msg}")
            .expect("Invalid indicatif progress bar template")
            .progress_chars("##-"),
    );
    progress
}

// --- TileDownloadCoordinator ---
// Not deriving Debug because Throttler doesn't derive Debug
pub(crate) struct TileDownloadCoordinator<'a> {
    downloader: TileDownloader,
    throttler: Throttler,
    args: &'a Arguments,
}

impl<'a> TileDownloadCoordinator<'a> {
    pub(crate) fn new(args: &'a Arguments) -> Result<Self, ZoomError> {
        let downloader = create_tile_downloader(args)?;
        let throttler = Throttler::new(args.min_interval);

        Ok(Self {
            downloader,
            throttler,
            args,
        })
    }

    pub(crate) async fn download_tiles(
        &mut self,
        tile_specs: impl IntoIterator<Item = Result<TileSpec, TileSourceError>>,
        canvas: &mut TileBuffer,
        state: &mut DownloadState,
        progress: &ProgressManager,
        canvas_size: Option<Vec2d>,
    ) -> Result<Vec<ObservationResult>, ZoomError> {
        progress.set_requesting_tiles();

        prepare_canvas_size(canvas, canvas_size)?;

        if canvas.prefers_encoded_tiles() {
            self.download_encoded_batch(tile_specs, canvas, state, progress)
                .await
        } else {
            self.download_decoded_batch(tile_specs, canvas, state, progress, canvas_size)
                .await
        }
    }
    async fn download_decoded_batch(
        &mut self,
        tile_specs: impl IntoIterator<Item = Result<TileSpec, TileSourceError>>,
        canvas: &mut TileBuffer,
        state: &mut DownloadState,
        progress: &ProgressManager,
        canvas_size: Option<Vec2d>,
    ) -> Result<Vec<ObservationResult>, ZoomError> {
        let mut stream = futures::stream::iter(tile_specs)
            .map(|tile_spec| {
                let downloader = &self.downloader;
                async move {
                    let tile_spec = tile_spec.map_err(WorkError::Source)?;
                    downloader
                        .download_tile_and_then(tile_spec, |downloaded| async move {
                            let spec = downloaded.spec.clone();
                            let tile = tokio::task::spawn_blocking(move || {
                                load_tile_with_metadata(spec.destination, &downloaded.bytes)
                            })
                            .await?
                            .map_err(ZoomError::from)?;
                            Ok((spec, tile))
                        })
                        .await
                        .map_err(|error| WorkError::Download(*error))
                }
            })
            .buffer_unordered(self.args.parallelism.max(1));

        let mut observations = Vec::new();
        while let Some(tile_result) = stream.next().await {
            debug!("Received tile result: {tile_result:?}");
            progress.increment();
            let (spec, tile, success) = match tile_result {
                Ok((spec, tile)) => {
                    let success = probe_succeeded(spec.role, tile.size());
                    if success {
                        state.set_tile_size(tile.size());
                    }
                    let keep_tile = success && spec.role != TileRole::Probe;
                    (spec, keep_tile.then_some(tile), success)
                }
                Err(WorkError::Download(error)) => {
                    let spec = error.tile_spec;
                    let tile = (spec.role == TileRole::Output)
                        .then(|| {
                            empty_tile_for(
                                spec.destination,
                                spec.expected_size.or(state.tile_size),
                                canvas_size,
                            )
                        })
                        .flatten();
                    (spec, tile, false)
                }
                Err(WorkError::Source(error)) => {
                    return Err(ZoomError::Dezoomer {
                        message: error.to_string(),
                    });
                }
            };
            if spec.role != TileRole::Output {
                observations.push(if success {
                    ObservationResult::Available {
                        size: state.tile_size.unwrap(),
                    }
                } else {
                    ObservationResult::Missing
                });
            }
            progress.update_for_tile(tile.as_ref(), success);
            if success {
                match spec.role {
                    TileRole::Output => state.record_output_success(),
                    TileRole::Probe => {}
                    TileRole::ProbeAndOutput => state.record_probe_output_success(),
                }
            }
            if let Some(tile) = tile {
                canvas.add_tile(tile).await;
            }
            self.throttler.wait().await;
        }
        Ok(observations)
    }

    async fn download_encoded_batch(
        &mut self,
        tile_specs: impl IntoIterator<Item = Result<TileSpec, TileSourceError>>,
        canvas: &mut TileBuffer,
        state: &mut DownloadState,
        progress: &ProgressManager,
    ) -> Result<Vec<ObservationResult>, ZoomError> {
        let mut stream = futures::stream::iter(tile_specs)
            .map(|tile_spec| {
                let downloader = &self.downloader;
                async move {
                    let tile_spec = tile_spec.map_err(WorkError::Source)?;
                    downloader
                        .download_tile_and_then(tile_spec, |downloaded| async move {
                            let spec = downloaded.spec.clone();
                            let tile = tokio::task::spawn_blocking(move || {
                                load_encoded_tile(spec.destination, downloaded.bytes)
                            })
                            .await?
                            .map_err(ZoomError::from)?;
                            Ok((spec, tile))
                        })
                        .await
                        .map_err(|error| WorkError::Download(*error))
                }
            })
            .buffer_unordered(self.args.parallelism.max(1));

        let mut observations = Vec::new();
        while let Some(tile_result) = stream.next().await {
            debug!("Received encoded tile result: {tile_result:?}");
            progress.increment();

            let (spec, tile, success) = match tile_result {
                Ok((spec, tile)) => {
                    let success = probe_succeeded(spec.role, tile.size);
                    if success {
                        state.set_tile_size(tile.size);
                    }
                    let keep_tile = success && spec.role != TileRole::Probe;
                    (spec, keep_tile.then_some(tile), success)
                }
                Err(WorkError::Download(error)) => (error.tile_spec, None, false),
                Err(WorkError::Source(error)) => {
                    return Err(ZoomError::Dezoomer {
                        message: error.to_string(),
                    });
                }
            };
            if spec.role != TileRole::Output {
                observations.push(if success {
                    ObservationResult::Available {
                        size: state.tile_size.unwrap(),
                    }
                } else {
                    ObservationResult::Missing
                });
            }
            progress.update_for_encoded_tile(tile.as_ref(), success);
            if success {
                match spec.role {
                    TileRole::Output => state.record_output_success(),
                    TileRole::Probe => {}
                    TileRole::ProbeAndOutput => state.record_probe_output_success(),
                }
            }
            if let Some(tile) = tile {
                canvas.add_encoded_tile(tile).await;
            }
            self.throttler.wait().await;
        }
        Ok(observations)
    }
}

fn probe_succeeded(role: TileRole, size: Vec2d) -> bool {
    role == TileRole::Output || size != Vec2d::square(1)
}

#[derive(Debug)]
enum WorkError {
    Source(TileSourceError),
    Download(TileDownloadError),
}

// Helper function, private to this module
fn create_tile_downloader(args: &Arguments) -> Result<TileDownloader, ZoomError> {
    Ok(TileDownloader {
        http_client: network_client(args.headers(), args, None)?,
        retries: args.retries,
        retry_delay: args.retry_delay,
        tile_storage_folder: args.tile_storage_folder.clone(),
        user_header_names: user_header_names(args.headers()),
        client_cache_headers: client_cache_headers(args),
    })
}

// Helper function, private to this module
fn prepare_canvas_size(
    canvas: &mut TileBuffer,
    canvas_size: Option<Vec2d>,
) -> Result<(), ZoomError> {
    if !canvas.has_size()
        && let Some(size) = canvas_size
    {
        canvas.set_size(size)?;
    }
    Ok(())
}

// Helper function, private to this module
fn empty_tile_for(
    position: Vec2d,
    tile_size: Option<Vec2d>,
    canvas_size: Option<Vec2d>,
) -> Option<Tile> {
    match (tile_size, canvas_size) {
        (Some(current_tile_size), Some(current_canvas_size)) => {
            let size = max_size_in_rect(position, current_tile_size, current_canvas_size);
            Some(Tile::empty(position, size))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ProgressManager, empty_tile_for, probe_succeeded};
    use crate::max_size_in_rect;
    use dezoomify_core::Vec2d;
    use dezoomify_core::core::TileRole;

    #[test]
    fn empty_tiles_are_clipped_to_canvas() {
        let tile_size = Vec2d { x: 256, y: 256 };
        let canvas_size = Vec2d { x: 1000, y: 1000 };
        let empty = empty_tile_for(Vec2d { x: 900, y: 900 }, Some(tile_size), Some(canvas_size));
        assert_eq!(
            empty.expect("replacement should fit").size(),
            Vec2d { x: 100, y: 100 }
        );
        assert_eq!(
            max_size_in_rect(Vec2d { x: 900, y: 900 }, tile_size, canvas_size),
            Vec2d { x: 100, y: 100 }
        );
    }

    #[test]
    fn missing_probe_and_output_is_not_a_partial_output() {
        let mut state = super::DownloadState::new();
        assert!(!state.has_partial_failure());
        state.record_probe_output_success();
        assert_eq!(state.total_tiles, 1);
        assert_eq!(state.successful_tiles, 1);
        assert!(!state.has_partial_failure());
    }

    #[test]
    fn one_by_one_probe_tiles_are_missing_placeholders() {
        assert!(!probe_succeeded(TileRole::ProbeAndOutput, Vec2d::square(1)));
        assert!(!probe_succeeded(TileRole::Probe, Vec2d::square(1)));
        assert!(probe_succeeded(TileRole::Output, Vec2d::square(1)));
        assert!(probe_succeeded(
            TileRole::ProbeAndOutput,
            Vec2d::square(256)
        ));
    }

    #[test]
    fn resolved_progress_discards_out_of_grid_probes() {
        let progress = ProgressManager::new();
        progress.increment();
        progress.increment();
        progress.set_resolved_tiles(4, 1);
        assert_eq!(progress.progress.position(), 1);
        assert_eq!(progress.progress.length(), Some(4));
    }
}
