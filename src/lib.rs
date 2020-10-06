use std::{fs, fmt, io};
use std::io::BufRead;
use std::path::PathBuf;
use std::time::Duration;

use futures::stream::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use itertools::Itertools;
use log::{debug, info, warn};
use reqwest::Client;

pub use arguments::Arguments;
use dezoomer::{PostProcessFn, TileFetchResult, ZoomLevel, ZoomLevelIter};
use dezoomer::{Dezoomer, DezoomerError, DezoomerInput, ZoomLevels};
use dezoomer::TileReference;
pub use errors::ZoomError;
use network::{client, fetch_uri};
use output_file::get_outname;
use tile::Tile;
pub use vec2d::Vec2d;

use crate::encoder::tile_buffer::TileBuffer;
use crate::output_file::reserve_output_file;
use crate::dezoomer::PageContents;
use std::error::Error;
use serde::export::Formatter;

mod arguments;
mod encoder;
pub mod dezoomer;
pub mod tile;
mod vec2d;
mod errors;
mod output_file;
mod network;

pub mod auto;
pub mod custom_yaml;
pub mod dzi;
pub mod generic;
pub mod google_arts_and_culture;
pub mod iiif;
pub mod pff;
pub mod zoomify;
pub mod krpano;
pub mod iipimage;
mod json_utils;

fn stdin_line() -> Result<String, ZoomError> {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let first_line = lines.next().ok_or_else(|| {
        let err_msg = "Encountered end of standard input while reading a line";
        io::Error::new(io::ErrorKind::UnexpectedEof, err_msg)
    })?;
    Ok(first_line?)
}

async fn list_tiles(
    dezoomer: &mut dyn Dezoomer,
    http: &Client,
    uri: &str,
) -> Result<ZoomLevels, ZoomError> {
    let mut i = DezoomerInput {
        uri: String::from(uri),
        contents: PageContents::Unknown,
    };
    loop {
        match dezoomer.zoom_levels(&i) {
            Ok(levels) => return Ok(levels),
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

/// An interactive level picker
fn level_picker(mut levels: Vec<ZoomLevel>) -> Result<ZoomLevel, ZoomError> {
    println!("Found the following zoom levels:");
    for (i, level) in levels.iter().enumerate() {
        println!("{}. {}", i, level.name());
    }
    loop {
        println!("Which level do you want to download? ");
        let line = stdin_line()?;
        if let Ok(idx) = line.parse::<usize>() {
            if levels.get(idx).is_some() {
                return Ok(levels.swap_remove(idx));
            }
        }
        println!("'{}' is not a valid level number", line);
    }
}

fn choose_level(mut levels: Vec<ZoomLevel>, args: &Arguments) -> Result<ZoomLevel, ZoomError> {
    match levels.len() {
        0 => Err(ZoomError::NoLevels),
        1 => Ok(levels.swap_remove(0)),
        _ => {
            let pos = args
                .best_size(levels.iter().filter_map(|l| l.size_hint()))
                .and_then(|best_size| {
                    levels
                        .iter()
                        .find_position(|&l| l.size_hint() == Some(best_size))
                });
            if let Some((i, _)) = pos {
                Ok(levels.swap_remove(i))
            } else {
                level_picker(levels)
            }
        }
    }
}

fn display_err<T, E: std::fmt::Display>(res: Result<T, E>) -> Option<T> {
    match res {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("{}", e);
            None
        }
    }
}

fn progress_bar(n: usize) -> ProgressBar {
    let progress = ProgressBar::new(n as u64);
    progress.set_style(
        ProgressStyle::default_bar()
            .template("[ETA:{eta}] {bar:40.cyan/blue} {pos:>4}/{len:4} {msg}")
            .progress_chars("##-"),
    );
    progress
}

async fn find_zoomlevel(args: &Arguments) -> Result<ZoomLevel, ZoomError> {
    let mut dezoomer = args.find_dezoomer()?;
    let uri = args.choose_input_uri()?;
    let http_client = client(args.headers(), args, Some(&uri))?;
    info!("Trying to locate a zoomable image...");
    let zoom_levels: Vec<ZoomLevel> = list_tiles(dezoomer.as_mut(), &http_client, &uri).await?;
    info!("Found {} zoom levels", zoom_levels.len());
    choose_level(zoom_levels, args)
}

pub async fn dezoomify(args: &Arguments) -> Result<PathBuf, ZoomError> {
    let zoom_level = find_zoomlevel(&args).await?;
    let outname = get_outname(&args.outfile, &zoom_level.title(), zoom_level.size_hint());
    let save_as = fs::canonicalize(outname.as_path()).unwrap_or_else(|_e| outname.clone());
    reserve_output_file(&save_as)?;
    let tile_buffer: TileBuffer = TileBuffer::new(save_as.clone(), args.compression).await?;
    info!("Dezooming {}", zoom_level.name());
    dezoomify_level(args, zoom_level, tile_buffer).await?;
    Ok(save_as)
}

pub async fn dezoomify_level(
    args: &Arguments,
    mut zoom_level: ZoomLevel,
    tile_buffer: TileBuffer,
) -> Result<(), ZoomError> {
    let level_headers = zoom_level.http_headers();
    let http_client = client(level_headers.iter().chain(args.headers()), &args, None)?;

    info!("Creating canvas");
    let mut canvas = tile_buffer;

    let progress = progress_bar(0);
    let mut total_tiles = 0u64;
    let mut successful_tiles = 0u64;

    let post_process_fn = zoom_level.post_process_fn();

    progress.set_message("Computing the URLs of the image tiles...");

    let mut zoom_level_iter = ZoomLevelIter::new(&mut zoom_level);
    let mut last_count = 0;
    let mut last_successes = 0;
    while let Some(tile_refs) = zoom_level_iter.next_tile_references() {
        last_count = tile_refs.len() as u64;
        total_tiles += last_count;
        progress.set_length(total_tiles);

        progress.set_message("Requesting the tiles...");

        let &Arguments { retries, retry_delay, .. } = args;
        let mut stream = futures::stream::iter(tile_refs)
            .map(|tile_ref: TileReference|
                download_tile(post_process_fn, tile_ref, &http_client, retries, retry_delay))
            .buffer_unordered(args.parallelism);

        last_successes = 0;
        let mut tile_size = None;

        if let Some(size) = zoom_level_iter.size_hint() {
            canvas.set_size(size).await?;
        }

        while let Some(tile_result) = stream.next().await {
            debug!("Received tile result: {:?}", tile_result);
            progress.inc(1);
            let tile = match tile_result {
                Ok(tile) => {
                    progress.set_message(&format!("Downloaded tile at {}", tile.position()));
                    tile_size.replace(tile.size());
                    last_successes += 1;
                    Some(tile)
                }
                Err(err) => {
                    // If a tile download fails, we replace it with an empty tile
                    progress.set_message(&err.to_string());
                    let position = err.tile_reference.position;
                    tile_size.and_then(|tile_size| {
                        zoom_level_iter.size_hint().map(|canvas_size| {
                            let size = max_size_in_rect(position, tile_size, canvas_size);
                            Tile::empty(position, size)
                        })
                    })
                }
            };
            if let Some(tile) = tile { display_err(canvas.add_tile(tile).await); }
        }
        successful_tiles += last_successes;
        zoom_level_iter.set_fetch_result(TileFetchResult {
            count: last_count,
            successes: last_successes,
            tile_size,
        });
    }

    progress.set_message("Downloaded all tiles. Finalizing the image file.");
    canvas.finalize().await?;

    progress.finish_with_message("Finished tile download");
    if successful_tiles == 0 { return Err(ZoomError::NoTile); }

    if last_successes < last_count {
        Err(ZoomError::PartialDownload { successful_tiles, total_tiles })
    } else {
        Ok(())
    }
}

async fn download_tile(
    post_process_fn: PostProcessFn,
    tile_reference: TileReference,
    client: &reqwest::Client,
    retries: usize,
    retry_delay: Duration,
) -> Result<Tile, TileDownloadError> {
    let mut res = Tile::download(post_process_fn, &tile_reference, client).await;
    // The initial delay after which a failed request is retried depends on the position of the tile
    // in order to avoid sending repeated "bursts" of requests to a server that is struggling
    let n = 100;
    let idx: f64 = ((tile_reference.position.x + tile_reference.position.y) % n).into();
    let mut wait_time = retry_delay + Duration::from_secs_f64(idx * retry_delay.as_secs_f64() / n as f64);
    for _ in 0..retries {
        res = Tile::download(post_process_fn, &tile_reference, client).await;
        match &res {
            Ok(_) => { break; },
            Err(e) => {
                warn!("{}. Retrying tile download in {:?}.", e, wait_time);
                tokio::time::delay_for(wait_time).await;
                wait_time *= 2;
            }
        }
    }
    res.map_err(|cause| TileDownloadError { tile_reference, cause })
}

#[derive(Debug)]
struct TileDownloadError {
    tile_reference: TileReference,
    cause: ZoomError,
}

impl fmt::Display for TileDownloadError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Unable to download tile '{}'. Cause: {}", self.tile_reference.url, self.cause)
    }
}

impl Error for TileDownloadError {}

/// Returns the maximal size a tile can have in order to fit in a canvas of the given size
pub fn max_size_in_rect(position: Vec2d, tile_size: Vec2d, canvas_size: Vec2d) -> Vec2d {
    (position + tile_size).min(canvas_size) - position
}
