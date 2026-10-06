//! # lmx
//!
//! Command of the LimaNix guest owner. People run it inside the VM; the LimaNix host runs it over
//! management SSH with `--json` and reads the [host contract](https://github.com/limanix/lmx/blob/main/docs/contract.md).
//!
//! | Command               | Kind   | Answers or does                                             |
//! |-----------------------|--------|-------------------------------------------------------------|
//! | `lmx help`            | facts  | the workspace and the commands inside the VM and on the Mac |
//! | `lmx info`            | facts  | kernel, guest disk, shared folders and failed units         |
//! | `lmx welcome`         | caller | the summary an interactive shell shows when it starts       |
//! | `lmx status`          | facts  | generations, disk, interfaces and failed units              |
//! | `lmx version`         | facts  | the binary version and the host contract it speaks          |
//! | `lmx clipboard copy`  | caller | copies standard input to the Mac clipboard                  |
//! | `lmx clipboard paste` | caller | prints the Mac clipboard, if the terminal allows reads      |
//! | `lmx session NAME`    | caller | opens a named session with the selected provider            |
//!
//! Facts are read in the caller's process with the caller's privileges and need no daemon. Caller
//! commands act on the caller's terminal and environment, so only the caller can run them.
//!
//! ## Other names
//!
//! Started under the name of a shell command it replaces, the binary runs that command. The
//! arguments, messages and exit statuses stay those of the replaced command.
//!
//! | Name                   | Runs                  |
//! |------------------------|-----------------------|
//! | `pbcopy`               | `lmx clipboard copy`  |
//! | `pbpaste`              | `lmx clipboard paste` |
//! | `limanix-session NAME` | `lmx session NAME`    |
//!
//! ## Test hooks
//!
//! Environment variables let tests point the binary at prepared files. `sudo` drops all of them by
//! default, so the host never sets them by accident.
//!
//! | Variable            | Replaces                                               |
//! |---------------------|--------------------------------------------------------|
//! | `LMX_CONFIG`        | [`lmx_model::CONFIG_PATH`]                             |
//! | `LMX_SYSTEM_ROOT`   | `/` for generation markers, the store path and `/proc` |
//! | `LMX_TTY_IN`        | `/dev/tty` for reading the terminal's clipboard reply  |
//! | `LMX_TTY_OUT`       | `/dev/tty` for writing clipboard sequences             |
//! | `LMX_PASTE_TIMEOUT` | the 10 seconds `pbpaste` waits for a reply, in seconds |
#![forbid(unsafe_code)]

mod cli;
mod clipboard;
mod format;
mod help;
mod info;
mod layout;
mod output;
mod palette;
mod process;
mod session;
mod status;
mod system;
mod version;
mod welcome;

use std::{
    env,
    ffi::{OsStr, OsString},
    io, iter,
    path::Path,
    process::ExitCode,
};

use clap::Parser;

use crate::{
    cli::{Cli, ClipboardCommand, Command},
    system::System,
};

fn main() -> ExitCode {
    let mut arguments = env::args_os();
    let program = arguments.next().unwrap_or_default();
    let arguments: Vec<OsString> = arguments.collect();

    let result = match Path::new(&program).file_name().and_then(OsStr::to_str) {
        Some("pbcopy") if arguments.is_empty() => clipboard::copy(),
        Some("pbcopy") => Ok(output::usage(clipboard::COPY_USAGE)),
        Some("pbpaste") if arguments.is_empty() => clipboard::paste(),
        Some("pbpaste") => Ok(output::usage(clipboard::PASTE_USAGE)),
        Some("limanix-session") => Ok(match arguments.as_slice() {
            [name] => session::run(&System::from_environment().config, name),
            _ => output::usage(session::USAGE),
        }),
        _ => lmx(arguments),
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

/// Runs `lmx` with the arguments after the program name.
fn lmx(mut arguments: Vec<OsString>) -> io::Result<ExitCode> {
    // `lmx --help` and `lmx -h` are `lmx help`, so further arguments stay usage errors, as in the
    // platform's shell `lmx`; `lmx status --help` stays generated.
    if let Some(first) = arguments.first_mut()
        && matches!(first.to_str(), Some("--help" | "-h"))
    {
        *first = OsString::from("help");
    }
    let cli = Cli::parse_from(iter::once(OsString::from("lmx")).chain(arguments));

    match cli.command {
        None | Some(Command::Help) => help::run(&System::from_environment()),
        Some(Command::Info) => info::run(&System::from_environment()),
        Some(Command::Welcome) => welcome::run(&System::from_environment()),
        Some(Command::Status(args)) => status::run(&System::from_environment(), &args),
        Some(Command::Version(args)) => version::run(&args),
        Some(Command::Clipboard(ClipboardCommand::Copy)) => clipboard::copy(),
        Some(Command::Clipboard(ClipboardCommand::Paste)) => clipboard::paste(),
        Some(Command::Session(args)) => {
            Ok(session::run(&System::from_environment().config, &args.name))
        }
    }
}
