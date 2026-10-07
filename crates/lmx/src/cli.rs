//! Command-line interface of `lmx`.
//!
//! The other names of the binary keep the syntax of the commands they replaced and are parsed in
//! `main`; `lmx`, `lmx --help` and `lmx -h` print the guest help page instead of the generated one.

use std::{ffi::OsString, time::Duration};

use clap::{Arg, ArgAction, Args, Parser, Subcommand, ValueEnum};

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
    Status(StatusArgs),
    /// Show the lmx version and the host contract it speaks.
    Version(OutputArgs),
    /// Keep room in the Nix store; the work runs in lmxd.
    #[command(subcommand)]
    Store(StoreCommand),
    /// Build the generation the host mounted for the next boot; the work runs in lmxd.
    Apply(ApplyArgs),
    /// Use the Mac clipboard through the terminal; also installed as pbcopy and pbpaste.
    #[command(subcommand)]
    Clipboard(ClipboardCommand),
    /// Open a named session with the selected provider; also installed as limanix-session.
    Session(SessionArgs),
}

/// Arguments of `lmx status`.
#[derive(Debug, Args)]
pub(crate) struct StatusArgs {
    /// Wait until the guest reaches this state, then answer; the host waits so after a restart.
    #[arg(long, value_enum, requires = "generation")]
    pub(crate) wait: Option<Goal>,
    /// Generation the wait is for.
    #[arg(short, long, requires = "wait")]
    pub(crate) generation: Option<String>,
    /// Longest wait, such as 90s, 10m or 1h [default: 10m].
    #[arg(long, value_parser = duration, requires = "wait")]
    pub(crate) timeout: Option<Duration>,
    /// Output selection.
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

/// A state `lmx status --wait` waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Goal {
    /// The generation is booted, healthy and finalized.
    Converged,
}

/// Arguments of `lmx apply`.
#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub(crate) struct ApplyArgs {
    /// Another apply operation.
    #[command(subcommand)]
    pub(crate) command: Option<ApplyCommand>,
    /// Generation to apply; it must be the one the host mounted.
    #[arg(short, long, required = true)]
    pub(crate) generation: Option<String>,
    /// Follow the apply until it ends, with the build's output.
    #[arg(long)]
    pub(crate) follow: bool,
    /// Output selection.
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

/// Apply operations besides starting one.
#[derive(Debug, Subcommand)]
pub(crate) enum ApplyCommand {
    /// Cancel the apply of a generation and wait until it stops.
    Cancel(CancelArgs),
}

/// Arguments of `lmx apply cancel`.
#[derive(Debug, Args)]
pub(crate) struct CancelArgs {
    /// Generation whose apply to cancel.
    #[arg(short, long)]
    pub(crate) generation: String,
    /// Output selection.
    #[command(flatten)]
    pub(crate) output: OutputArgs,
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

/// Parses a duration such as `90s`, `10m` or `1h`; a number alone is seconds.
fn duration(text: &str) -> Result<Duration, String> {
    let digits = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(digits);
    let scale = match unit {
        "" | "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        _ => 0,
    };
    match number.parse::<u64>() {
        Ok(number) if number > 0 && scale > 0 => {
            Ok(Duration::from_secs(number.saturating_mul(scale)))
        }
        _ => Err("expected a duration such as 90s, 10m or 1h".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_durations_in_seconds_minutes_and_hours() {
        assert_eq!(duration("90"), Ok(Duration::from_secs(90)));
        assert_eq!(duration("90s"), Ok(Duration::from_secs(90)));
        assert_eq!(duration("10m"), Ok(Duration::from_secs(600)));
        assert_eq!(duration("1h"), Ok(Duration::from_secs(3600)));
        for invalid in ["", "0s", "m", "10 m", "1.5h", "-1s", "10d"] {
            assert!(duration(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn apply_takes_a_generation_or_a_cancel() {
        let cli = Cli::try_parse_from(["lmx", "apply", "-g", "g1", "--follow", "--json"])
            .expect("an apply");
        let Some(Command::Apply(args)) = cli.command else {
            panic!("not an apply");
        };
        assert_eq!(args.generation.as_deref(), Some("g1"));
        assert!(args.follow && args.output.json && args.command.is_none());

        let cli = Cli::try_parse_from(["lmx", "apply", "cancel", "--generation", "g1"])
            .expect("a cancel");
        let Some(Command::Apply(ApplyArgs {
            command: Some(ApplyCommand::Cancel(cancel)),
            ..
        })) = cli.command
        else {
            panic!("not a cancel");
        };
        assert_eq!(cancel.generation, "g1");

        assert!(Cli::try_parse_from(["lmx", "apply"]).is_err());
        assert!(Cli::try_parse_from(["lmx", "apply", "cancel"]).is_err());
        assert!(Cli::try_parse_from(["lmx", "status", "--wait", "converged"]).is_err());
        assert!(Cli::try_parse_from(["lmx", "status", "--timeout", "1m"]).is_err());
    }
}
