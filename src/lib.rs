use std::fs;
use std::io::BufRead;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use itertools::Itertools;
use reqwest::Client;

pub use arguments::Arguments;
use canvas::{Canvas, Tile};
use dezoomer::{PostProcessFn, TileFetchResult, ZoomLevel, ZoomLevelIter};
use dezoomer::{Dezoomer, DezoomerError, DezoomerInput, ZoomLevels};
use dezoomer::TileReference;
use network::{fetch_uri, client};
pub use errors::ZoomError;
use output_file::get_outname;
pub use vec2d::Vec2d;

use crate::output_file::reserve_output_file;

mod arguments;
mod canvas;
pub mod dezoomer;
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

fn stdin_line() -> String {
    std::io::stdin()
        .lock()
        .lines()
        .next()
        .expect("Invalid input")
        .expect("Unable to read from stdin")
}

async fn list_tiles(
    dezoomer: &mut dyn Dezoomer,
    http: &Client,
    uri: &str,
) -> Result<ZoomLevels, ZoomError> {
    let mut i = DezoomerInput {
        uri: String::from(uri),
        contents: None,
    };
    loop {
        match dezoomer.zoom_levels(&i) {
            Ok(levels) => return Ok(levels),
            Err(DezoomerError::NeedsData { uri }) => {
                let contents = fetch_uri(&uri, http).await?;
                i.uri = uri;
                i.contents = Some(contents);
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
        let line = stdin_line();
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
    let uri = args.choose_input_uri();
    let http_client = client(args.headers(), args)?;
    println!("Trying to locate a zoomable image...");
    let zoom_levels: Vec<ZoomLevel> = list_tiles(dezoomer.as_mut(), &http_client, &uri).await?;
    choose_level(zoom_levels, args)
}

pub async fn dezoomify(args: &Arguments) -> Result<(), ZoomError> {
    if let Some(path) = &args.outfile {
        reserve_output_file(path)?;
    }
    let mut zoom_level = find_zoomlevel(&args).await?;
    println!("Dezooming {}", zoom_level.name());

    let level_headers = zoom_level.http_headers();
    let http_client = client(level_headers.iter().chain(args.headers()), &args)?;

    let canvas = Arc::new(Mutex::new(Canvas::new(zoom_level.size_hint())));

    let progress = progress_bar(0);
    let mut total_tiles = 0u64;
    let mut successful_tiles = 0u64;

    let post_process_fn = zoom_level.post_process_fn();

    progress.set_message("Computing the URLs of the image tiles...");

    let mut zoom_level_iter = ZoomLevelIter::new(&mut zoom_level);
    while let Some(tile_refs) = zoom_level_iter.next() {
        let count = tile_refs.len() as u64;
        total_tiles += count;
        progress.set_length(total_tiles);

        progress.set_message("Requesting the tiles...");

        let &Arguments { retries, retry_delay, .. } = args;
        let mut stream = futures::stream::iter(&tile_refs)
            .map(|tile_ref: &TileReference|
                download_tile(post_process_fn, tile_ref, &http_client, retries, retry_delay))
            .buffer_unordered(args.parallelism);

        let mut successes = 0;
        let mut tile_size = None;

        while let Some(tile_result) = stream.next().await {
            progress.inc(1);
            match tile_result {
                Ok(tile) => {
                    progress.set_message(&format!("Downloaded tile at {}", tile.position()));
                    tile_size.replace(tile.size());
                    let canvas = Arc::clone(&canvas);
                    tokio::spawn(async move {
                        tokio::task::block_in_place(move || {
                            display_err(canvas.lock().unwrap().add_tile(&tile));
                        })
                    }).await?;
                    successes += 1;
                }
                Err(e) => {
                    progress.set_message(&e.to_string());
                }
            }
        }
        successful_tiles += successes;
        zoom_level_iter.set_fetch_result(TileFetchResult { count, successes, tile_size });
    }

    progress.finish_with_message("Finished tile download");
    if successful_tiles == 0 { return Err(ZoomError::NoTile); }

    let canvas = canvas.lock().unwrap();
    let outname = get_outname(&args.outfile, &zoom_level.title());
    println!("Saving the image to {}...", outname.to_string_lossy());
    let save_as = fs::canonicalize(outname.as_path()).unwrap_or(outname);
    canvas.image().save(save_as.as_path())?;
    let saved_as = save_as.to_string_lossy();
    println!("Saved the image to {}", &saved_as);
    if successful_tiles < total_tiles {
        let saved_as = saved_as.to_string();
        Err(ZoomError::PartialDownload { successful_tiles, total_tiles, saved_as })
    } else {
        Ok(())
    }
}

async fn download_tile(
    post_process_fn: PostProcessFn,
    tile_reference: &TileReference,
    client: &reqwest::Client,
    retries: usize,
    retry_delay: Duration,
) -> Result<Tile, ZoomError> {
    let mut res = Tile::download(post_process_fn, tile_reference, client).await;
    // The initial delay after which a failed request is retried depends on the position of the tile
    // in order to avoid sending repeated "bursts" of requests to a server that is struggling
    let n = 100;
    let idx: f64 = ((tile_reference.position.x + tile_reference.position.y) % n).into();
    let mut wait_time = retry_delay + Duration::from_secs_f64(idx * retry_delay.as_secs_f64() / n as f64);
    for _ in 0..retries {
        res = Tile::download(post_process_fn, tile_reference, client).await;
        if res.is_ok() { break; }
        tokio::time::delay_for(wait_time).await;
        wait_time *= 2;
    }
    res.map_err(|e| ZoomError::TileDownloadError {
        uri: tile_reference.url.clone(),
        cause: e.into(),
    })
}