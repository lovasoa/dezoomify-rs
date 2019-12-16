use structopt::StructOpt;

use crate::dezoomer::Dezoomer;

use super::{auto, stdin_line, Vec2d, ZoomError};
use std::time::Duration;

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
    #[structopt(short = "n", long = "num-downloads", default_value = "16")]
    pub num_threads: usize,

    /// Number of new attempts to make when a tile load fails
    /// before giving up. Setting this to 0 is useful to speed up the
    /// generic dezoomer, which relies on failed tile loads to detect the
    /// dimensions of the image. On the contrary, if a server is not reliable,
    /// set this value to a higher number.
    #[structopt(short = "r", long = "retries", default_value = "1")]
    pub retries: usize,

    /// Amount of time to wait before retrying a request that failed
    #[structopt(long = "retry-delay", default_value = "2s", parse(try_from_str = "parse_duration"))]
    pub retry_delay: Duration,

    /// Sets an HTTP header to use on requests.
    /// This option can be repeated in order to set multiple headers.
    /// You can use `-H "Referer: URL"` where URL is the URL of the website's
    /// viewer page in order to let the site think you come from a the legitimate viewer.
    #[structopt(
    short = "H",
    long = "header",
    parse(try_from_str = "parse_header"),
    number_of_values = 1
    )]
    headers: Vec<(String, String)>,

    /// Maximum number of idle connections per host allowed at the same time
    #[structopt(long = "max-idle-per-host", default_value = "64")]
    pub max_idle_per_host: usize,

    /// Whether to accept connecting to insecure HTTPS servers
    #[structopt(long = "accept-invalid-certs")]
    pub accept_invalid_certs: bool,

    /// Maximum time between the beginning of a request and the end of a response before
    ///the request should be interrupted and considered considered failed
    #[structopt(long = "timeout", default_value = "30s", parse(try_from_str = "parse_duration"))]
    pub timeout: Duration,

    /// Time after which we should give up when trying to connect to a server
    #[structopt(long = "connect-timeout", default_value = "6s", parse(try_from_str = "parse_duration"))]
    pub connect_timeout: Duration,
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
                    self.max_width.map(|w| s.x <= w).unwrap_or(true)
                        && self.max_height.map(|h| s.y <= h).unwrap_or(true)
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

fn parse_duration(s: &str) -> Result<Duration, &'static str> {
    let val: u64 = s.chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .map_err(|_| "Invalid duration value")?;
    let unit = s.chars()
        .skip_while(|c| c.is_ascii_digit() || c.is_whitespace())
        .collect::<String>();
    match &unit[..] {
        "min" => Ok(Duration::from_secs(60 * val)),
        "s" => Ok(Duration::from_secs(val)),
        "ms" => Ok(Duration::from_millis(val)),
        "ns" => Ok(Duration::from_nanos(val)),
        _ => Err("Invalid duration unit")
    }
}


#[test]
fn test_headers_and_input() -> Result<(), structopt::clap::Error> {
    let args: Arguments = StructOpt::from_iter_safe(
        [
            "dezoomify-rs",
            "--header",
            "Referer: http://test.com",
            "--header",
            "User-Agent: custom",
            "--header",
            "A:B",
            "input-url",
        ]
        .iter(),
    )?;
    assert_eq!(args.input_uri, Some("input-url".into()));
    assert_eq!(
        args.headers,
        vec![
            ("Referer".into(), "http://test.com".into()),
            ("User-Agent".into(), "custom".into()),
            ("A".into(), "B".into()),
        ]
    );
    Ok(())
}

#[test]
fn test_parse_duration() {
    assert_eq!(Ok(Duration::from_secs(2)), parse_duration("2s"));
    assert_eq!(Ok(Duration::from_secs(29)), parse_duration("29 s"));
    assert_eq!(Ok(Duration::from_secs(120)), parse_duration("2min"));
    assert_eq!(Ok(Duration::from_secs(1)), parse_duration("1000 ms"));
    assert!(parse_duration("ms").is_err());
    assert!(parse_duration("1j").is_err());
    assert!(parse_duration("").is_err());
}