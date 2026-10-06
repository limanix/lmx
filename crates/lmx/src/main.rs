//! # lmx
//!
//! Command of the LimaNix guest owner. People run it inside the VM; the LimaNix host runs it over
//! management SSH with `--json` and reads the [host contract](https://github.com/limanix/lmx/blob/main/docs/contract.md).
//!
//! | Command       | Kind  | Answers                                                     |
//! |---------------|-------|-------------------------------------------------------------|
//! | `lmx status`  | facts | generations, disk, interfaces and failed units              |
//! | `lmx version` | facts | the binary version and the host contract it speaks          |
//!
//! Facts are read in the caller's process with the caller's privileges and need no daemon.
//!
//! ## Test hooks
//!
//! Two environment variables let tests point the binary at prepared files. `sudo` drops both by
//! default, so the host never sets them by accident.
//!
//! | Variable          | Replaces                                                   |
//! |-------------------|------------------------------------------------------------|
//! | `LMX_CONFIG`      | [`lmx_model::CONFIG_PATH`]                                 |
//! | `LMX_SYSTEM_ROOT` | `/` for generation markers and the store path              |
#![forbid(unsafe_code)]

mod cli;
mod format;
mod output;
mod status;
mod system;
mod version;

use std::{io, process::ExitCode};

use clap::Parser;

use crate::{
    cli::{Cli, Command},
    system::System,
};

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Command::Status(args)) => status::run(&System::from_environment(), &args),
        Some(Command::Version(args)) => version::run(&args),
        None => cli::print_help(),
    };

    match result {
        Ok(code) => code,
        // Standard output was closed by its reader, as in `lmx status | true`: nobody is left to
        // read an answer or an error. clap ends `--help` the same way.
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lmx: {error}");
            ExitCode::from(output::FAILURE)
        }
    }
}
