//! Command-line entry point: argument parsing, logging setup and the run loop.

use env_logger::TimestampPrecision;
use human_panic::setup_panic;

use dezoomify_rs::{Arguments, ZoomError, dezoomify, process_bulk};
use log::{error, info, warn};

#[tokio::main]
async fn main() {
    setup_panic!();
    let has_args = std::env::args_os().count() > 1;
    let mut has_errors = false;
    let args: Arguments = clap::Parser::parse();
    init_log(&args);

    if args.is_bulk_mode() {
        // Bulk processing mode
        match process_bulk(&args).await {
            Ok(stats) => {
                if stats.failed_images > 0 || stats.partial_downloads > 0 {
                    has_errors = true;
                }
            }
            Err(err) => {
                error!("{err}");
                has_errors = true;
            }
        }
    } else {
        // Single processing mode (existing behavior)
        loop {
            match dezoomify(&args).await {
                Ok(saved_as) => {
                    info!(
                        "Image successfully saved to '{}'",
                        saved_as.to_string_lossy(),
                    );
                }
                Err(ZoomError::Io { source })
                    if source.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    // If we have reached the end of stdin, we exit
                    warn!("Reached end of input. Exiting...");
                    break;
                }
                Err(err @ ZoomError::PartialDownload { .. }) => {
                    warn!("{err}");
                    has_errors = true;
                }
                Err(err) => {
                    error!("{err}");
                    has_errors = true;
                }
            }
            if has_args {
                // Command-line invocation
                break;
            }
        }
    }
    if has_errors {
        std::process::exit(1);
    }
}

fn init_log(args: &Arguments) {
    let logging = &args.logging;
    let is_default_logging = logging.eq_ignore_ascii_case("info");
    let env = env_logger::Env::new().default_filter_or(logging);
    env_logger::Builder::from_env(env)
        .format_timestamp(if is_default_logging {
            None
        } else {
            Some(TimestampPrecision::Millis)
        })
        .format_target(!is_default_logging)
        .init();
}
