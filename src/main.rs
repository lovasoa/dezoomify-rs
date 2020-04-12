use dezoomify_rs::{Arguments, dezoomify};
use colour::{green_ln, red_ln};
use human_panic::setup_panic;
use structopt::StructOpt;
use env_logger;

#[tokio::main]
async fn main() {
    setup_panic!();
    let has_args = std::env::args_os().count() > 1;
    let mut has_errors = false;
    let args: Arguments = Arguments::from_args();
    init_log(&args);

    loop {
        if let Err(err) = dezoomify(&args).await {
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

fn init_log(args: &Arguments) {
    let env = env_logger::Env::new().default_filter_or(&args.logging);
    env_logger::init_from_env(env);
}