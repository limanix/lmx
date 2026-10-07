//! Command-line interface of `lmx`.
//!
//! The other names of the binary keep the syntax of the commands they replaced and are parsed in
//! `main`; `lmx`, `lmx --help` and `lmx -h` print the guest help page instead of the generated one.

use std::ffi::OsString;

use clap::{Arg, ArgAction, Args, Parser, Subcommand};

/// Guest owner command of a LimaNix VM.
#[derive(Debug, Parser)]
#[command(name = "lmx", version, disable_help_subcommand = true)]
pub(crate) struct Cli {
    /// Command to run; without one, `lmx` prints the guest help page.
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

/// Commands of `lmx`.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Show the workspace and the commands inside this VM and on the Mac.
    // `lmx help --help` and `lmx help -h` print the page too, as the platform's shell `lmx` did.
    #[command(
        disable_help_flag = true,
        arg = Arg::new("help").short('h').long("help").action(ArgAction::SetTrue).hide(true)
    )]
    Help,
    /// Show the kernel, guest disk, shared folders and failed units.
    Info,
    /// Show the workspace welcome again.
    Welcome,
    /// Show generations, disk usage, network interfaces and failed units.
    Status(OutputArgs),
    /// Show the lmx version and the host contract it speaks.
    Version(OutputArgs),
    /// Keep room in the Nix store; the work runs in lmxd.
    #[command(subcommand)]
    Store(StoreCommand),
    /// Use the Mac clipboard through the terminal; also installed as pbcopy and pbpaste.
    #[command(subcommand)]
    Clipboard(ClipboardCommand),
    /// Open a named session with the selected provider; also installed as limanix-session.
    Session(SessionArgs),
}

/// Store operations.
#[derive(Debug, Subcommand)]
pub(crate) enum StoreCommand {
    /// Collect unreferenced store paths when free space is low; the host runs it before it stops
    /// the VM.
    Reserve(OutputArgs),
}

/// Clipboard operations.
#[derive(Debug, Subcommand)]
pub(crate) enum ClipboardCommand {
    /// Copy standard input to the Mac clipboard.
    Copy,
    /// Print the Mac clipboard, if the terminal allows reads.
    Paste,
}

/// Arguments of `lmx session`.
#[derive(Debug, Args)]
pub(crate) struct SessionArgs {
    /// Session name, passed to the provider unchanged; put -- before a name that starts with -.
    pub(crate) name: OsString,
}

/// Output selection shared by commands that answer the host.
#[derive(Debug, Args)]
pub(crate) struct OutputArgs {
    /// Answer with the JSON host contract instead of text.
    #[arg(long)]
    pub(crate) json: bool,
}
