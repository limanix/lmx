//! Command-line interface.

use std::{io, process::ExitCode};

use clap::{Args, CommandFactory, Parser, Subcommand};

/// Guest owner command of a LimaNix VM.
#[derive(Debug, Parser)]
#[command(name = "lmx", version, disable_help_subcommand = true)]
pub(crate) struct Cli {
    /// Command to run; without one, `lmx` prints help.
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

/// Commands of `lmx`.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Show generations, disk usage, network interfaces and failed units.
    Status(OutputArgs),
    /// Show the lmx version and the host contract it speaks.
    Version(OutputArgs),
}

/// Output selection shared by commands that answer the host.
#[derive(Debug, Args)]
pub(crate) struct OutputArgs {
    /// Answer with the JSON host contract instead of text.
    #[arg(long)]
    pub(crate) json: bool,
}

/// Prints the generated help when `lmx` runs without a command.
pub(crate) fn print_help() -> io::Result<ExitCode> {
    Cli::command().print_help()?;
    Ok(ExitCode::SUCCESS)
}
