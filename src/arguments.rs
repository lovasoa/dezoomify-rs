use structopt::StructOpt;

use crate::dezoomer::Dezoomer;

use super::{auto, stdin_line, Vec2d, ZoomError};

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
    /// before giving up. Setting this to 0 is useful to speed up the
    /// generic dezoomer, which relies on failed tile loads to detect the
    /// dimensions of the image. On the contrary, if a server is not reliable,
    /// set this value to a higher number.
    #[structopt(short = "r", long = "retries", default_value = "1")]
    pub retries: usize,

    /// Sets an HTTP header to use on requests.
    /// This option can be repeated in order to set multiple headers.
    /// You can use `-H "Referer: URL"` where URL is the URL of the website's
    /// viewer page in order to let the site think you come from a the legitimate viewer.
    #[structopt(short = "H", long = "header", parse(try_from_str = "parse_header"), number_of_values = 1)]
    headers: Vec<(String, String)>,
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
    pub fn best_size<I: Iterator<Item = Vec2d>>(&self, sizes: I) -> Option<Vec2d> {
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

    pub fn headers(&self) -> impl Iterator<Item = (&String, &String)> {
        self.headers.iter().map(|(k, v)| (k, v))
    }
}

fn parse_header(s: &str) -> Result<(String, String), &'static str> {
    let vals: Vec<&str> = s.splitn(2, ':').map(str::trim).collect();
    if let [key, value] = vals[..] {
        Ok((key.into(), value.into()))
    } else {
        Err("Invalid header format. Expected 'Name: Value'")
    }
}

#[test]
fn test_headers_and_input() -> Result<(), structopt::clap::Error> {
    let args: Arguments = StructOpt::from_iter_safe([
        "dezoomify-rs",
        "--header", "Referer: http://test.com",
        "--header", "User-Agent: custom",
        "--header", "A:B",
        "input-url"
    ].iter())?;
    assert_eq!(args.input_uri, Some("input-url".into()));
    assert_eq!(args.headers, vec![
        ("Referer".into(), "http://test.com".into()),
        ("User-Agent".into(), "custom".into()),
        ("A".into(), "B".into()),
    ]);
    Ok(())
}