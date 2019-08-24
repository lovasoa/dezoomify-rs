use structopt::StructOpt;

use crate::{auto, stdin_line, Vec2d, ZoomError};
use crate::dezoomer::Dezoomer;

#[derive(StructOpt, Debug)]
pub struct Arguments {
    /// Input URL or local file name
    input_uri: Option<String>,

    /// File to which the resulting image should be saved
    #[structopt(default_value = "dezoomified.jpg")]
    pub outfile: std::path::PathBuf,

    /// Name of the dezoomer to use
    #[structopt(short = "d", long = "dezoomer", default_value = "auto")]
    dezoomer: String,

    /// If several zoom levels are available, then select the largest one
    #[structopt(short = "l")]
    largest: bool,

    /// If several zoom levels are available, then select the one with the largest width that
    /// is inferior to max-width.
    #[structopt(short = "w", long = "max-width")]
    max_width: Option<u32>,

    /// If several zoom levels are available, then select the one with the largest height that
    /// is inferior to max-height.
    #[structopt(short = "h", long = "max-height")]
    max_height: Option<u32>,

    /// Degree of parallelism to use. At most this number of
    /// tiles will be downloaded at the same time.
    #[structopt(short = "n", long = "num-threads")]
    pub num_threads: Option<usize>,

    /// Number of new attempts to make when a tile load fails
    /// before abandoning
    #[structopt(short = "r", long = "retries", default_value = "1")]
    pub retries: usize,
}

impl Arguments {
    pub fn choose_input_uri(&self) -> String {
        match &self.input_uri {
            Some(uri) => uri.clone(),
            None => {
                println!("Enter an URL or a path to a tiles.yaml file: ");
                stdin_line()
            }
        }
    }
    pub fn find_dezoomer(&self) -> Result<Box<dyn Dezoomer>, ZoomError> {
        auto::all_dezoomers(true)
            .into_iter()
            .find(|d| d.name() == self.dezoomer)
            .ok_or_else(|| ZoomError::NoSuchDezoomer {
                name: self.dezoomer.clone(),
            })
    }
    pub fn best_size<I: Iterator<Item=Vec2d>>(&self, sizes: I) -> Option<Vec2d> {
        if self.largest {
            sizes.max_by_key(|s| s.x * s.y)
        } else if self.max_width.is_some() || self.max_height.is_some() {
            sizes
                .filter(|s| {
                    self.max_width.map(|w| s.x < w).unwrap_or(true)
                        && self.max_height.map(|h| s.y < h).unwrap_or(true)
                })
                .max_by_key(|s| s.x * s.y)
        } else {
            None
        }
    }
}