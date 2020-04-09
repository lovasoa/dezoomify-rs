use dezoomify_rs::{Arguments, dezoomify};
use colour::{green_ln, red_ln};
use human_panic::setup_panic;
use structopt::StructOpt;

#[tokio::main]
async fn main() {
    setup_panic!();
    let has_args = std::env::args_os().count() > 1;
    let mut has_errors = false;
    let conf: Arguments = Arguments::from_args();
    loop {
        if let Err(err) = dezoomify(&conf).await {
            red_ln!("ERROR {}", err);
            has_errors = true;
        } else {
            green_ln!("Done!");
        }
        if has_args {
            // Command-line invocation
            break;
        }
    }
    if has_errors {
        std::process::exit(1);
    }
}