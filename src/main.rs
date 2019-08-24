use std::collections::HashMap;
use std::error::Error;
use std::io::{BufRead, Read};
use std::sync::Mutex;
use std::time::Duration;
use std::{fs, thread};

use indicatif::{ProgressBar, ProgressStyle};
use itertools::Itertools;
use rayon::prelude::*;
use rayon::ThreadPoolBuilder;
use reqwest::{header, Client};
use structopt::StructOpt;

use canvas::{Canvas, Tile};
use custom_error::custom_error;
use dezoomer::TileReference;
use dezoomer::{Dezoomer, DezoomerError, DezoomerInput, ZoomLevels};
pub use vec2d::Vec2d;

use dezoomer::{apply_to_tiles, PostProcessFn, TileFetchResult, ZoomLevel};
use arguments::Arguments;

mod canvas;
mod dezoomer;
mod vec2d;
mod arguments;

mod auto;
mod custom_yaml;
mod generic;
mod google_arts_and_culture;
mod iiif;
mod zoomify;
mod dzi;


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

fn main() {
    let conf: Arguments = Arguments::from_args();
    if let Err(err) = dezoomify(conf) {
        eprintln!("{}", err);
        std::process::exit(1);
    } else {
        println!("Done!");
    }
}

fn fetch_uri(uri: &str, http: &Client) -> Result<Vec<u8>, ZoomError> {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        println!("Downloading {}...", uri);
        let mut contents = Vec::new();
        let mut response = http.get(uri).send()?.error_for_status()?;
        response.read_to_end(&mut contents)?;
        Ok(contents)
    } else {
        println!("Opening {}...", uri);
        Ok(fs::read(uri)?)
    }
}

fn list_tiles(
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
                let contents = fetch_uri(&uri, http)?;
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

fn find_zoomlevel(args: &Arguments) -> Result<ZoomLevel, ZoomError> {
    let mut dezoomer = args.find_dezoomer()?;
    let uri = args.choose_input_uri();
    let http_client = client(HashMap::new())?;
    println!("Trying to locate a zoomable image...");
    let zoom_levels: Vec<ZoomLevel> = list_tiles(dezoomer.as_mut(), &http_client, &uri)?;
    choose_level(zoom_levels, args)
}

fn dezoomify(args: Arguments) -> Result<(), ZoomError> {
    initialize_threadpool(&args);
    let mut zoom_level = find_zoomlevel(&args)?;
    println!("Dezooming {}", zoom_level.name());

    let http_client = client(zoom_level.http_headers())?;

    let canvas = Mutex::new(Canvas::new(zoom_level.size_hint()));

    let progress = progress_bar(0);
    let mut total_tiles = 0u64;
    let mut successful_tiles = 0u64;

    let post_process_fn = zoom_level.post_process_fn();

    apply_to_tiles(&mut zoom_level, |tile_refs| {
        let count = tile_refs.len() as u64;
        total_tiles += count;
        progress.set_length(total_tiles);

        let (successes, tile_size) = tile_refs
            .into_par_iter()
            .map(|tile_ref: TileReference| {
                progress.inc(1);
                progress.set_message(&format!("Downloading tile at {}", tile_ref.position));
                let tile = download_tile(post_process_fn, &tile_ref, &http_client, args.retries);
                let res = tile.and_then(|tile| {
                    canvas.lock().unwrap().add_tile(&tile)?;
                    Ok(tile.size())
                });
                display_err(res)
                    .map(|size| (1, Some(size)))
                    .unwrap_or((0, None))
            })
            .reduce(|| (0, None), |a, b| (a.0 + b.0, a.1.or(b.1)));
        successful_tiles += successes;
        TileFetchResult {
            count,
            successes,
            tile_size,
        }
    });

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

    let canvas = canvas.into_inner().unwrap();

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

fn download_tile(
    post_process_fn: Option<PostProcessFn>,
    tile_reference: &TileReference,
    client: &reqwest::Client,
    retries: usize,
) -> Result<Tile, ZoomError> {
    let mut res = Tile::download(post_process_fn, tile_reference, client);
    let mut wait_time = Duration::from_millis(100);
    for _ in 0..retries {
        thread::sleep(wait_time);
        wait_time *= 2;
        res = Tile::download(post_process_fn, tile_reference, client);
        match &res {
            Ok(_) => break,
            Err(e) => eprintln!("{}", e),
        }
    }
    res.map_err(|e| ZoomError::TileDownloadError {
        uri: tile_reference.url.clone(),
        cause: e.into(),
    })
}

fn client(headers: HashMap<String, String>) -> Result<reqwest::Client, ZoomError> {
    let header_map: Result<header::HeaderMap, ZoomError> = default_headers()
        .iter()
        .chain(headers.iter())
        .map(|(name, value)| Ok((name.parse()?, value.parse()?)))
        .collect();
    let client = reqwest::Client::builder()
        .default_headers(header_map?)
        .build()?;
    Ok(client)
}

fn initialize_threadpool(args: &Arguments) {
    let mut builder = ThreadPoolBuilder::new();
    if let Some(num_threads) = args.num_threads {
        builder = builder.num_threads(num_threads)
    }
    builder = builder.thread_name(|i| format!("dezoomify-rs thread {}", i));
    builder
        .build_global()
        .expect("threadpool initialization failed");
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
}
