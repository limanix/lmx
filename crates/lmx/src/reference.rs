//! `reference.json`: the commands of `lmx`, what they report and need, and how `lmx` exits, for the
//! LimaNix site.

use std::{env, fs, path::Path};

use clap::{Arg, Command, CommandFactory, builder::StyledStr};
use lmx_model::CheckStatus;
use serde::Serialize;

use crate::{
    apply,
    checks::{self, CheckId},
    cli::Cli,
    output, palette, session,
    status::{Line, Word},
};

const COMMAND_CHECKS: [(&str, &[CheckId]); 2] = [
    ("lmx doctor", &checks::DOCTOR),
    ("lmx net check", &checks::NET),
];

const OWNER_OPERATION: &str = "Root, with `sudo`, and a running `lmxd`";

const PRIVILEGES: [(&str, &str); 8] = [
    (
        "lmx status",
        "Nothing. With `sudo` it also reads the generation the Mac selected; `--wait` needs a \
         running `lmxd`",
    ),
    (
        "lmx doctor",
        "Nothing. With `sudo` it also reads the generation the Mac selected",
    ),
    (
        "lmx net check",
        "Nothing. With `sudo` it also sees the processes of other users",
    ),
    (
        "lmx logs",
        "Root, or membership in the `wheel` or `systemd-journal` group, which may read the \
         journal of `lmxd`",
    ),
    ("lmx store reserve", OWNER_OPERATION),
    ("lmx apply", OWNER_OPERATION),
    ("lmx apply cancel", OWNER_OPERATION),
    (
        "lmx dotfiles link",
        "The development account. It refuses any other user, root included",
    ),
];

const EXIT_STATUSES: [(u8, &str); 6] = [
    (0, "Success"),
    (
        output::FAILURE,
        "The command failed, or a check of `lmx doctor` or `lmx net check` failed",
    ),
    (output::USAGE, "Usage error"),
    (
        output::UNAVAILABLE,
        "`lmxd` is unavailable. Only `store reserve`, `apply`, `apply cancel` and \
         `status --wait` need it",
    ),
    (
        session::NO_PROVIDER,
        "`lmx session` or `limanix-session` found no session provider",
    ),
    (apply::CANCELLED, "The apply was cancelled"),
];

const ENVIRONMENT: [(&str, &str); 2] = [
    (palette::NO_COLOR, "Set and not empty: no colors"),
    (palette::TERM, "Unset, empty or `dumb`: no colors"),
];

const CHECK_STATUSES: [CheckStatus; 4] = [
    CheckStatus::Ok,
    CheckStatus::Warning,
    CheckStatus::Failed,
    CheckStatus::Unknown,
];

#[derive(Serialize)]
struct Reference {
    commands: Vec<Entry>,
    exit_statuses: Vec<ExitStatus>,
    environment: Vec<Topic>,
    check_statuses: Vec<Topic>,
    status_lines: Vec<Topic>,
    short_words: Vec<Topic>,
}

#[derive(Serialize)]
struct Entry {
    name: String,
    usage: String,
    about: String,
    details: Option<String>,
    privileges: Option<&'static str>,
    args: Vec<Argument>,
    checks: Vec<Topic>,
}

#[derive(Serialize)]
struct Argument {
    name: String,
    short: Option<String>,
    value: Option<String>,
    required: bool,
    default: Option<String>,
    values: Vec<Choice>,
    about: String,
    details: Option<String>,
}

#[derive(Serialize)]
struct Choice {
    name: String,
    about: String,
}

#[derive(Serialize)]
struct Topic {
    id: String,
    what: String,
}

#[derive(Serialize)]
struct ExitStatus {
    status: u8,
    what: &'static str,
}

fn render() -> String {
    let mut built = Cli::command();
    built.build();
    let mut commands = Vec::new();
    walk(&Cli::command(), &built, None, &mut commands);
    let listed = COMMAND_CHECKS.iter().map(|(name, _)| *name);
    for name in listed.chain(PRIVILEGES.iter().map(|(name, _)| *name)) {
        assert!(
            commands.iter().any(|entry| entry.name == name),
            "no command {name}"
        );
    }
    let topic = |id: &str, what: String| Topic {
        id: id.to_owned(),
        what,
    };
    let reference = Reference {
        commands,
        exit_statuses: EXIT_STATUSES
            .into_iter()
            .map(|(status, what)| ExitStatus { status, what })
            .collect(),
        environment: ENVIRONMENT
            .into_iter()
            .map(|(name, what)| topic(name, what.into()))
            .collect(),
        check_statuses: CHECK_STATUSES
            .into_iter()
            .map(|status| topic(&status_name(status), meaning(status)))
            .collect(),
        status_lines: Line::ALL
            .into_iter()
            .map(|line| topic(line.name(), line.what().into()))
            .collect(),
        short_words: Word::ALL
            .into_iter()
            .map(|word| topic(word.name(), word.what()))
            .collect(),
    };
    let mut json = serde_json::to_string_pretty(&reference).expect("a reference");
    json.push('\n');
    json
}

fn status_name(status: CheckStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .expect("a status name")
}

fn meaning(status: CheckStatus) -> String {
    match status {
        CheckStatus::Ok => "Fine".into(),
        CheckStatus::Warning => "Worth knowing; nothing is broken".into(),
        CheckStatus::Failed => format!(
            "Broken. The command exits with status `{}`",
            output::FAILURE
        ),
        CheckStatus::Unknown => {
            "Your account cannot read what the check needs. Run the command with `sudo`".into()
        }
    }
}

fn walk(command: &Command, built: &Command, parent: Option<&str>, commands: &mut Vec<Entry>) {
    let name = parent.map_or_else(
        || command.get_name().to_owned(),
        |parent| format!("{parent} {}", command.get_name()),
    );
    let checks = COMMAND_CHECKS
        .iter()
        .filter(|(listed, _)| *listed == name)
        .flat_map(|(_, checks)| checks.iter())
        .map(|check| Topic {
            id: check.name().to_owned(),
            what: check.what().to_owned(),
        })
        .collect();
    commands.push(Entry {
        name: name.clone(),
        usage: usage(built),
        about: text(command.get_about()),
        details: command.get_long_about().map(ToString::to_string),
        privileges: PRIVILEGES
            .iter()
            .find(|(listed, _)| *listed == name)
            .map(|(_, needs)| *needs),
        args: command
            .get_arguments()
            .filter(|arg| !arg.is_hide_set())
            .map(argument)
            .collect(),
        checks,
    });
    for subcommand in command.get_subcommands().filter(|sub| !sub.is_hide_set()) {
        let built = built
            .find_subcommand(subcommand.get_name())
            .expect("a built subcommand");
        walk(subcommand, built, Some(&name), commands);
    }
}

fn usage(command: &Command) -> String {
    let usage = command.clone().render_usage().to_string();
    usage
        .trim_start_matches("Usage: ")
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
}

fn argument(arg: &Arg) -> Argument {
    let value = arg
        .get_value_names()
        .and_then(<[_]>::first)
        .map_or_else(|| arg.get_id().as_str().to_uppercase(), ToString::to_string);
    let takes_value = arg.get_action().takes_values();
    let values = if takes_value {
        arg.get_possible_values()
    } else {
        Vec::new()
    };
    Argument {
        name: arg
            .get_long()
            .map_or_else(|| value.clone(), |long| format!("--{long}")),
        short: arg.get_short().map(|short| format!("-{short}")),
        value: (arg.get_long().is_some() && takes_value).then_some(value),
        required: arg.is_required_set(),
        default: arg
            .get_default_values()
            .first()
            .map(|default| default.to_string_lossy().into_owned()),
        values: values
            .iter()
            .filter(|possible| !possible.is_hide_set())
            .map(|possible| Choice {
                name: possible.get_name().to_owned(),
                about: text(possible.get_help()),
            })
            .collect(),
        about: text(arg.get_help()),
        details: arg.get_long_help().map(ToString::to_string),
    }
}

fn text(styled: Option<&StyledStr>) -> String {
    styled.map(ToString::to_string).unwrap_or_default()
}

#[test]
fn reference_json_matches_the_commands() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference.json");
    let reference = render();
    if env::var_os("LMX_UPDATE_REFERENCE").is_some() {
        fs::write(&path, &reference).expect("write reference.json");
    }
    let saved = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        saved == reference,
        "reference.json is out of date: run task reference/fix"
    );
}
