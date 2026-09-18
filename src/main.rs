//! cargo-msrv-pin: downgrade Cargo dependencies to versions compatible with
//! a given toolchain.
//!
//! Finds every registry dependency whose declared MSRV exceeds the target
//! toolchain and downgrades it to the highest still-compatible version,
//! relaxing a blocking Cargo.toml requirement when a plain downgrade isn't
//! enough, then verifies the result with `cargo check`.
//!
//! Invoke either directly (`cargo-msrv-pin <toolchain>`) or as a cargo
//! subcommand (`cargo msrv-pin <toolchain>`).

mod command;
mod downgrade;
mod error;
mod lockfile;
mod manifest;
mod metadata;
mod registry;
mod version;

use clap::Parser;
use error::Error;
use std::env;
use std::io::Write;

#[derive(Parser)]
#[command(about = "Downgrade Cargo dependencies to versions compatible with a given toolchain")]
struct Args {
    /// The target toolchain (e.g. 1.75 or 1.75.0)
    toolchain: String,

    /// Change the working directory
    #[arg(short = 'C', value_name = "PATH")]
    working_directory: Option<String>,

    /// Show debug logs
    #[arg(short, long)]
    verbose: bool,
}

// Deliberately uses anyhow::Result here, not the app's own error::Result:
// it gives free message-plus-chain printing on failure.
fn main() -> anyhow::Result<()> {
    // Strip the `msrv-pin` subcommand so this binary can be invoked either
    // directly or as `cargo msrv-pin`.
    let mut raw_args: Vec<String> = env::args().collect();
    if raw_args.len() > 1 && raw_args[1] == "msrv-pin" {
        raw_args.remove(1);
    }

    let args = Args::parse_from(raw_args);

    env_logger::Builder::new()
        .filter_level(if args.verbose {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        })
        .format(|buf, record| {
            let level = record.level();
            let style = buf.default_level_style(level);
            writeln!(buf, "{style}{level}{style:#}: {}", record.args())
        })
        .init();

    if let Some(dir) = &args.working_directory {
        env::set_current_dir(dir).map_err(|source| Error::Io {
            action: "changing directory",
            source,
        })?;
    }

    downgrade::run(&args.toolchain)?;
    Ok(())
}
