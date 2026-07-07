// download_state.rs
use crate::arguments::Arguments;
use crate::dezoomer::{TileFetchResult, TileReference, ZoomLevel, ZoomLevelIter};
use crate::encoder::tile_buffer::TileBuffer;
use crate::errors::{self, ZoomError}; // `self` imports the errors module itself
use crate::max_size_in_rect;
use crate::network::{DownloadedTile, TileDownloader, client as network_client};
use crate::throttler::Throttler;
use crate::tile::{EncodedTile, Tile, load_encoded_tile, load_tile_with_metadata};
use crate::vec2d::Vec2d; // This is a public function from lib.rs

use futures::stream::StreamExt;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use log::debug;
use std::default::Default;

// --- DownloadState ---
#[derive(Debug, Default)]
pub(crate) struct DownloadState {
    pub(crate) total_tiles: u64,
    pub(crate) successful_tiles: u64,
    pub(crate) last_batch_count: u64,
    pub(crate) last_batch_successes: u64,
    tile_size: Option<Vec2d>,
}

impl DownloadState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn add_batch(&mut self, count: u64) {
        self.last_batch_count = count;
        self.total_tiles += count;
        self.last_batch_successes = 0;
    }

    pub(crate) fn record_success(&mut self) {
        self.last_batch_successes += 1;
        self.successful_tiles += 1;
    }

    fn set_tile_size(&mut self, size: Vec2d) {
        self.tile_size = Some(size);
    }

    pub(crate) fn create_fetch_result(&self) -> TileFetchResult {
        TileFetchResult {
            count: self.last_batch_count,
            successes: self.last_batch_successes,
            tile_size: self.tile_size,
        }
    }

    pub(crate) fn is_successful(&self) -> bool {
        self.successful_tiles > 0
    }

    pub(crate) fn has_partial_failure(&self) -> bool {
        self.last_batch_successes < self.last_batch_count
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

    pub(crate) fn update_for_tile(&self, tile: &Option<Tile>, success: bool) {
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

    pub(crate) fn update_for_encoded_tile(&self, tile: &Option<EncodedTile>, success: bool) {
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
    pub(crate) fn new(zoom_level: &ZoomLevel, args: &'a Arguments) -> Result<Self, ZoomError> {
        let downloader = create_tile_downloader(zoom_level, args)?;
        let throttler = Throttler::new(args.min_interval);

        Ok(Self {
            downloader,
            throttler,
            args,
        })
    }

    pub(crate) async fn download_batch(
        &mut self,
        tile_refs: Vec<TileReference>,
        canvas: &mut TileBuffer,
        state: &mut DownloadState,
        progress: &ProgressManager,
        zoom_level_iter: &ZoomLevelIter<'_>,
    ) -> Result<(), ZoomError> {
        state.add_batch(tile_refs.len() as u64);
        progress.set_total_tiles(state.total_tiles); // Update progress bar length with cumulative total
        progress.set_requesting_tiles();

        prepare_canvas_size(canvas, zoom_level_iter).await?;

        if canvas.prefers_encoded_tiles() {
            self.download_encoded_batch(tile_refs, canvas, state, progress)
                .await
        } else {
            self.download_decoded_batch(tile_refs, canvas, state, progress, zoom_level_iter)
                .await
        }
    }
    async fn download_decoded_batch(
        &mut self,
        tile_refs: Vec<TileReference>,
        canvas: &mut TileBuffer,
        state: &mut DownloadState,
        progress: &ProgressManager,
        zoom_level_iter: &ZoomLevelIter<'_>,
    ) -> Result<(), ZoomError> {
        let mut stream = futures::stream::iter(tile_refs)
            .map(|tile_ref: TileReference| self.downloader.download_tile(tile_ref))
            .buffer_unordered(self.args.parallelism);

        while let Some(tile_result) = stream.next().await {
            debug!("Received tile result: {:?}", tile_result);
            progress.increment();

            let (tile, success) = process_downloaded_tile_result(
                tile_result,
                &mut state.tile_size,
                zoom_level_iter.size_hint(),
            )
            .await?;

            progress.update_for_tile(&tile, success);

            if success {
                state.record_success();
                if let Some(ref tile) = tile {
                    state.set_tile_size(tile.size());
                }
            }

            if let Some(tile) = tile {
                canvas.add_tile(tile).await;
            }
            self.throttler.wait().await;
        }
        Ok(())
    }

    async fn download_encoded_batch(
        &mut self,
        tile_refs: Vec<TileReference>,
        canvas: &mut TileBuffer,
        state: &mut DownloadState,
        progress: &ProgressManager,
    ) -> Result<(), ZoomError> {
        let mut stream = futures::stream::iter(tile_refs)
            .map(|tile_ref: TileReference| self.downloader.download_tile(tile_ref))
            .buffer_unordered(self.args.parallelism);

        while let Some(tile_result) = stream.next().await {
            debug!("Received encoded tile result: {:?}", tile_result);
            progress.increment();

            let (tile, success) = process_encoded_tile_result(tile_result).await?;

            progress.update_for_encoded_tile(&tile, success);

            if success {
                state.record_success();
                if let Some(ref tile) = tile {
                    state.set_tile_size(tile.size);
                }
            }

            if let Some(tile) = tile {
                canvas.add_encoded_tile(tile).await;
            }
            self.throttler.wait().await;
        }
        Ok(())
    }
}

// Helper function, private to this module
fn create_tile_downloader(
    zoom_level: &ZoomLevel,
    args: &Arguments,
) -> Result<TileDownloader, ZoomError> {
    let level_headers = zoom_level.http_headers();
    Ok(TileDownloader {
        http_client: network_client(level_headers.iter().chain(args.headers()), args, None)?,
        post_process_fn: zoom_level.post_process_fn(),
        retries: args.retries,
        retry_delay: args.retry_delay,
        tile_storage_folder: args.tile_storage_folder.clone(),
    })
}

// Helper function, private to this module
async fn prepare_canvas_size(
    canvas: &mut TileBuffer,
    zoom_level_iter: &ZoomLevelIter<'_>,
) -> Result<(), ZoomError> {
    if !canvas.has_size() {
        if let Some(size) = zoom_level_iter.size_hint() {
            canvas.set_size(size).await?;
        }
    }
    Ok(())
}

// Helper function, private to this module
async fn process_downloaded_tile_result(
    tile_result: Result<DownloadedTile, errors::TileDownloadError>,
    tile_size: &mut Option<Vec2d>,
    canvas_size: Option<Vec2d>,
) -> Result<(Option<Tile>, bool), ZoomError> {
    match tile_result {
        Ok(downloaded) => {
            let tile = tokio::task::spawn_blocking(move || {
                load_tile_with_metadata(downloaded.position, &downloaded.bytes)
            })
            .await??;
            *tile_size = Some(tile.size());
            Ok((Some(tile), true))
        }
        Err(err) => {
            let position = err.tile_reference.position;
            let empty_tile = match (*tile_size, canvas_size) {
                (Some(current_tile_size), Some(current_canvas_size)) => {
                    let size = max_size_in_rect(position, current_tile_size, current_canvas_size);
                    Some(Tile::empty(position, size))
                }
                _ => None,
            };
            Ok((empty_tile, false))
        }
    }
}

async fn process_encoded_tile_result(
    tile_result: Result<DownloadedTile, errors::TileDownloadError>,
) -> Result<(Option<EncodedTile>, bool), ZoomError> {
    match tile_result {
        Ok(downloaded) => {
            let tile = tokio::task::spawn_blocking(move || {
                load_encoded_tile(downloaded.position, downloaded.bytes)
            })
            .await??;
            Ok((Some(tile), true))
        }
        Err(_) => Ok((None, false)),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use image::ImageEncoder;

    use super::process_downloaded_tile_result;
    use crate::dezoomer::TileReference;
    use crate::errors::{TileDownloadError, ZoomError};
    use crate::max_size_in_rect;
    use crate::network::DownloadedTile;
    use crate::vec2d::Vec2d;

    #[tokio::test]
    async fn test_process_downloaded_tile_result() {
        let mut tile_size: Option<Vec2d> = None;
        let canvas_size = Vec2d { x: 1000, y: 1000 };

        let mut encoded_png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut encoded_png)
            .write_image(
                &vec![255; 256 * 256 * 3],
                256,
                256,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        let ok_result = Ok(DownloadedTile {
            position: Vec2d { x: 0, y: 0 },
            bytes: Arc::new(encoded_png),
        });
        let (result_tile_opt, success) =
            process_downloaded_tile_result(ok_result, &mut tile_size, Some(canvas_size))
                .await
                .unwrap();

        assert!(success, "Tile processing should succeed for Ok result");
        assert!(
            result_tile_opt.is_some(),
            "Result tile should be Some for Ok result"
        );
        if let Some(ref result_tile) = result_tile_opt {
            assert_eq!(
                result_tile.size(),
                Vec2d { x: 256, y: 256 },
                "Result tile size mismatch"
            );
        }
        assert_eq!(
            tile_size,
            Some(Vec2d { x: 256, y: 256 }),
            "tile_size variable mismatch after success"
        );

        tile_size = Some(Vec2d { x: 256, y: 256 });

        let tile_ref = TileReference {
            url: "http://example.com/tile.jpg".to_string(),
            position: Vec2d { x: 100, y: 100 },
        };
        let error = TileDownloadError {
            tile_reference: tile_ref.clone(),
            cause: ZoomError::NoLevels,
        };
        let err_result = Err(error);
        let (result_tile_opt_err, success_err) =
            process_downloaded_tile_result(err_result, &mut tile_size, Some(canvas_size))
                .await
                .unwrap();

        assert!(!success_err, "Tile processing should fail for Err result");
        assert!(
            result_tile_opt_err.is_some(),
            "Result tile should be Some (empty tile) for Err result"
        );
        if let Some(ref empty_tile) = result_tile_opt_err {
            let expected_empty_size =
                max_size_in_rect(tile_ref.position, tile_size.unwrap(), canvas_size);
            assert_eq!(
                empty_tile.size(),
                expected_empty_size,
                "Empty tile size mismatch"
            );
            assert_eq!(
                empty_tile.position(),
                tile_ref.position,
                "Empty tile position mismatch"
            );
        }
        assert_eq!(
            tile_size,
            Some(Vec2d { x: 256, y: 256 }),
            "tile_size variable mismatch after failure"
        );
    }
}
