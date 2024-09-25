use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use regex::Regex;

use crate::dezoomer::Dezoomer;

use super::{auto, stdin_line, Vec2d, ZoomError};

#[derive(Parser, Debug)]
#[command(author, version, about, disable_help_flag = true)]
pub struct Arguments {
    /// Displays this help message
    #[arg(short = '?', long = "help", action = clap::ArgAction::Help)]
    pub display_help: (),

    /// Input URL or local file name. By default, the program will ask for it interactively.
    pub input_uri: Option<String>,

    /// File to which the resulting image should be saved. By default the program will
    /// generate a name based on the image metadata if available. Otherwise, it will
    /// generate a name in the format "dezoomified[_N].{jpg,png}" depending on which
    /// files already exist in the current directory, and whether the target image size fits
    /// in a JPEG or not.
    #[arg()]
    pub outfile: Option<PathBuf>,

    /// Name of the dezoomer to use
    #[arg(short, long, default_value = "auto")]
    dezoomer: String,

    /// If several zoom levels are available, then select the largest one
    #[arg(short, long)]
    pub largest: bool,

    /// If several zoom levels are available, then select the one with the largest width that
    /// is inferior to max-width.
    #[arg(short = 'w', long = "max-width")]
    max_width: Option<u32>,

    /// If several zoom levels are available, then select the one with the largest height that
    /// is inferior to max-height.
    #[arg(short = 'h', long = "max-height")]
    max_height: Option<u32>,

    /// Degree of parallelism to use. At most this number of
    /// tiles will be downloaded at the same time.
    #[arg(short = 'n', long = "parallelism", default_value = "16")]
    pub parallelism: usize,

    /// Number of new attempts to make when a tile load fails
    /// before giving up. Setting this to 0 is useful to speed up the
    /// generic dezoomer, which relies on failed tile loads to detect the
    /// dimensions of the image. On the contrary, if a server is not reliable,
    /// set this value to a higher number.
    #[arg(short = 'r', long = "retries", default_value = "1")]
    pub retries: usize,

    /// Amount of time to wait before retrying a request that failed.
    /// Applies only to the first retry. Subsequent retries follow an
    /// exponential backoff strategy: each one is twice as long as
    /// the previous one.
    #[arg(long, default_value = "2s", value_parser = parse_duration)]
    pub retry_delay: Duration,

    /// A number between 0 and 100 expressing how much to compress the output image.
    /// For lossy output formats such as jpeg, this affects the quality of the resulting image.
    /// 0 means less compression, 100 means more compression.
    /// Currently affects only the JPEG and PNG encoders.
    #[arg(long, default_value = "5")]
    pub compression: u8,

    /// Sets an HTTP header to use on requests.
    /// This option can be repeated in order to set multiple headers.
    /// You can use `-H "Referer: URL"` where URL is the URL of the website's
    /// viewer page in order to let the site think you come from the legitimate viewer.
    #[arg(
    short = 'H',
    long = "header",
    value_parser = parse_header,
    number_of_values = 1
    )]
    pub headers: Vec<(String, String)>,

    /// Maximum number of idle connections per host allowed at the same time
    #[arg(long, default_value = "32")]
    pub max_idle_per_host: usize,

    /// Whether to accept connecting to insecure HTTPS servers
    #[arg(long)]
    pub accept_invalid_certs: bool,

    /// Minimum amount of time to wait between two consequent requests.
    /// This throttles the flow of image tile requests coming from your computer,
    /// reducing the risk of crashing the remote server of getting banned for making too many
    /// requests in a short succession.
    #[arg(short = 'i', long, default_value = "50ms", value_parser = parse_duration)]
    pub min_interval: Duration,

    /// Maximum time between the beginning of a request and the end of a response before
    ///the request should be interrupted and considered failed
    #[arg(long, default_value = "30s", value_parser = parse_duration)]
    pub timeout: Duration,

    /// Time after which we should give up when trying to connect to a server
    #[arg(long = "connect-timeout", default_value = "6s", value_parser = parse_duration)]
    pub connect_timeout: Duration,

    /// Level of logging verbosity. Set it to "debug" to get all logging messages.
    #[arg(long, default_value = "warn")]
    pub logging: String,

    /// A place to store the image tiles when after they are downloaded and decrypted.
    /// By default, tiles are not stored to disk (which is faster), but using a tile cache allows
    /// retrying partially failed downloads, or stitching the tiles with an external program.
    #[arg(short = 'c', long = "tile-cache")]
    pub tile_storage_folder: Option<PathBuf>,
}

impl Default for Arguments {
    fn default() -> Self {
        Arguments {
            display_help: (),
            input_uri: None,
            outfile: None,
            dezoomer: "auto".to_string(),
            largest: false,
            max_width: None,
            max_height: None,
            parallelism: 16,
            retries: 1,
            compression: 20,
            retry_delay: Duration::from_secs(2),
            headers: vec![],
            max_idle_per_host: 32,
            accept_invalid_certs: false,
            min_interval: Default::default(),
            timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(6),
            logging: "warn".to_string(),
            tile_storage_folder: None,
        }
    }
}

impl Arguments {
    pub fn choose_input_uri(&self) -> Result<String, ZoomError> {
        match &self.input_uri {
            Some(uri) => Ok(uri.clone()),
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
            sizes.max_by_key(|s| s.area())
        } else if self.max_width.is_some() || self.max_height.is_some() {
            sizes
                .filter(|s| {
                    self.max_width.map(|w| s.x <= w).unwrap_or(true)
                        && self.max_height.map(|h| s.y <= h).unwrap_or(true)
                })
                .max_by_key(|s| s.area())
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

fn parse_duration(s: &str) -> Result<Duration, &'static str> {
    let err_msg = "Invalid duration. \
                        A duration is a number followed by a unit, such as '10ms' or '5s'";
    let re = Regex::new(r"^(\d+)\s*(min|s|ms|ns)$").unwrap();
    let caps = re.captures(s).ok_or(err_msg)?;
    let val: u64 = caps[1].parse().map_err(|_| err_msg)?;
    match &caps[2] {
        "h" => Ok(Duration::from_secs(3600 * val)),
        "min" | "m" => Ok(Duration::from_secs(60 * val)),
        "s" => Ok(Duration::from_secs(val)),
        "ms" => Ok(Duration::from_millis(val)),
        "ns" => Ok(Duration::from_nanos(val)),
        _ => Err(err_msg),
    }
}

#[test]
fn test_headers_and_input() {
    let args = Arguments::parse_from([
        "dezoomify-rs",
        //
        "--header",
        "Referer: http://test.com",
        //
        "--header",
        "User-Agent: custom",
        //
        "--header",
        "A:B",
        //
        "input-url",
    ]);
    assert_eq!(args.input_uri, Some("input-url".into()));
    assert_eq!(
        args.headers,
        vec![
            ("Referer".into(), "http://test.com".into()),
            ("User-Agent".into(), "custom".into()),
            ("A".into(), "B".into()),
        ]
    );
}

#[test]
fn test_parse_duration() {
    assert_eq!(parse_duration("2s"), Ok(Duration::from_secs(2)));
    assert_eq!(parse_duration("29 s"), Ok(Duration::from_secs(29)));
    assert_eq!(parse_duration("2min"), Ok(Duration::from_secs(120)));
    assert_eq!(parse_duration("1000 ms"), Ok(Duration::from_secs(1)));
    assert!(parse_duration("1 2 ms").is_err());
    assert!(parse_duration("1 s s").is_err());
    assert!(parse_duration("ms").is_err());
    assert!(parse_duration("1j").is_err());
    assert!(parse_duration("").is_err());
}
