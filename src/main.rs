use std::{fs, thread};
use std::collections::HashMap;
use std::error::Error;
use std::io::{BufRead, Read};
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use std::thread::spawn;
use std::time::Duration;

use futures::stream::{self, futures_unordered::FuturesUnordered, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use itertools::Itertools;
use regex::Replacer;
use reqwest::{Client, header};
use structopt::StructOpt;
use tokio::prelude::*;

use arguments::Arguments;
use canvas::{Canvas, Tile};
use custom_error::custom_error;
use dezoomer::{apply_to_tiles, PostProcessFn, TileFetchResult, ZoomLevel};
use dezoomer::{Dezoomer, DezoomerError, DezoomerInput, ZoomLevels};
use dezoomer::TileReference;
pub use vec2d::Vec2d;

use crate::canvas::WorkAround;

mod arguments;
mod canvas;
mod dezoomer;
mod vec2d;

mod auto;
mod custom_yaml;
mod dzi;
mod generic;
mod google_arts_and_culture;
mod iiif;
mod zoomify;

fn stdin_line() -> String {
    std::io::stdin()
        .lock()
        .lines()
        .next()
        .expect("Invalid input")
        .expect("Unable to read from stdin")
}

pub fn default_headers() -> HashMap<String, String> {
    serde_yaml::from_str(include_str!("default_headers.yaml")).unwrap()
}

#[tokio::main]
async fn main() {
    let conf: Arguments = Arguments::from_args();
    if let Err(err) = dezoomify(conf).await {
        eprintln!("{}", err);
        std::process::exit(1);
    } else {
        println!("Done!");
    }
}

// TODO: return Bytes
async fn fetch_uri(uri: &str, http: &Client) -> Result<Vec<u8>, ZoomError> {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        println!("Downloading {}...", uri);
        let mut response = http.get(uri).send().await?.error_for_status()?;
        let mut contents = Vec::new();
        contents.extend(response.bytes().await?);
        Ok(contents)
    } else {
        println!("Opening {}...", uri);
        Ok(fs::read(uri)?)
    }
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
    let http_client = client(args.headers())?;
    println!("Trying to locate a zoomable image...");
    let zoom_levels: Vec<ZoomLevel> = list_tiles(dezoomer.as_mut(), &http_client, &uri).await?;
    choose_level(zoom_levels, args)
}

async fn dezoomify(args: Arguments) -> Result<(), ZoomError> {
    let mut zoom_level = find_zoomlevel(&args).await?;
    println!("Dezooming {}", zoom_level.name());

    let level_headers = zoom_level.http_headers();
    let http_client = client(level_headers.iter().chain(args.headers()))?;

    let canvas = Arc::new(Mutex::new(Canvas::new(zoom_level.size_hint())));

    let progress = progress_bar(0);
    let mut total_tiles = 0u64;
    let mut successful_tiles = 0u64;

    let post_process_fn = WorkAround(zoom_level.post_process_fn());

    progress.set_message("Computing the URLs of the image tiles...");

    // TODO: support multiple next_tiles
    let start = std::time::Instant::now();
    let tile_refs = zoom_level.next_tiles(None);

    let count = tile_refs.len() as u64;
    total_tiles += count;
    progress.set_length(total_tiles);

    progress.set_message("Requesting the tiles...");

    let retries = args.retries;
    let mut stream = futures::stream::iter(&tile_refs)
        .map(|tile_ref: &TileReference| download_tile(post_process_fn, tile_ref, &http_client, retries))
        .buffer_unordered(args.num_threads);

    let mut successes = 0;
    let mut tile_size = None;

    while let Some(tile_result) = stream.next().await {
        progress.inc(1);
        if let Some(tile) = display_err(tile_result) {
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
    }
    successful_tiles += successes;
    TileFetchResult {
        count,
        successes,
        tile_size,
    };

    let final_msg = if successful_tiles == total_tiles {
        "Downloaded all tiles.".into()
    } else if successful_tiles > 0 {
        format!(
            "Successfully downloaded {} tiles out of {}",
            successful_tiles, total_tiles
        )
    } else {
        return Err(ZoomError::NoTile);
    };
    progress.finish_with_message(&final_msg);

    let canvas = canvas.lock().unwrap();

    println!("Saving the image to {}...", &args.outfile.to_string_lossy());
    canvas.image().save(&args.outfile)?;
    println!(
        "Saved the image to {}",
        fs::canonicalize(&args.outfile)
            .unwrap_or(args.outfile)
            .to_string_lossy()
    );
    Ok(())
}

async fn download_tile(
    post_process_fn: WorkAround,
    tile_reference: &TileReference,
    client: &reqwest::Client,
    retries: usize,
) -> Result<Tile, ZoomError> {
    let mut res = Tile::download(post_process_fn, tile_reference, client).await;
    let mut wait_time = Duration::from_millis(100);
    for _ in 0..retries {
        res = Tile::download(post_process_fn, tile_reference, client).await;
        match &res {
            Ok(_) => break,
            Err(e) => eprintln!("{}", e),
        }
        tokio::time::delay_for(wait_time).await;
        wait_time *= 2;
    }
    res.map_err(|e| ZoomError::TileDownloadError {
        uri: tile_reference.url.clone(),
        cause: e.into(),
    })
}

fn client<'a, I: Iterator<Item = (&'a String, &'a String)>>(
    headers: I,
) -> Result<reqwest::Client, ZoomError> {
    let header_map: Result<header::HeaderMap, ZoomError> = default_headers()
        .iter()
        .chain(headers.map(|(k, v)| (k, v)))
        .map(|(name, value)| Ok((name.parse()?, value.parse()?)))
        .collect();
    let client = reqwest::Client::builder()
        .default_headers(header_map?)
        .max_idle_per_host(64)
        .build()?;
    Ok(client)
}

custom_error! {
    pub ZoomError
    Networking{source: reqwest::Error} = "network error: {source}",
    Dezoomer{source: DezoomerError} = "Dezoomer error: {source}",
    NoLevels = "A zoomable image was found, but it did not contain any zoom level",
    NoTile = "Could not get any tile for the image",
    Image{source: image::ImageError} = "invalid image error: {source}",
    TileDownloadError{uri: String, cause: Box<ZoomError>} = "error with tile {uri}: {cause}",
    PostProcessing{source: Box<dyn Error>} = "unable to process the downloaded tile: {source}",
    Io{source: std::io::Error} = "Input/Output error: {source}",
    Yaml{source: serde_yaml::Error} = "Invalid YAML configuration file: {source}",
    TileCopyError{x:u32, y:u32, twidth:u32, theight:u32, width:u32, height:u32} =
                                "Unable to copy a {twidth}x{theight} tile \
                                 at position {x},{y} \
                                 on a canvas of size {width}x{height}",
    MalformedTileStr{tile_str: String} = "Malformed tile string: '{tile_str}' \
                                          expected 'x y url'",
    NoSuchDezoomer{name: String} = "No such dezoomer: {name}",
    InvalidHeaderName{source: header::InvalidHeaderName} = "Invalid header name: {source}",
    InvalidHeaderValue{source: header::InvalidHeaderValue} = "Invalid header value: {source}",
    AsyncError{source: tokio::task::JoinError} = "Unable get the result from a thread: {source}",
}
