#![forbid(unsafe_code)]

use std::process::ExitCode;

use clap::{error::ErrorKind, Parser};
use tokedb_runtime::cli::{self, Cli};
use tokedb_runtime::config::RuntimeConfig;
use tokedb_runtime::error::RuntimeError;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                    | ErrorKind::DisplayVersion
            ) =>
        {
            let _ = err.print();
            return ExitCode::SUCCESS;
        }
        Err(err) => err.exit(),
    };

    match RuntimeConfig::from_env() {
        Ok(_) => {}
        Err(err) => return fail(err),
    }

    match cli::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail(err),
    }
}

fn fail(err: RuntimeError) -> ExitCode {
    eprintln!("error: {err}");
    ExitCode::FAILURE
}
