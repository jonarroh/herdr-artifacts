use std::process::ExitCode;

use clap::Parser;
use herdr_artifacts::cli::{self, Cli};

fn main() -> ExitCode {
    cli::run(Cli::parse())
}
