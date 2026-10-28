//! `reference.json`: the commands of `lmx` and the checks they report, for the LimaNix site.

use std::{env, fs, path::Path};

use clap::{Arg, Command, CommandFactory, builder::StyledStr};
use serde::Serialize;

use crate::{
    checks::{self, CheckId},
    cli::Cli,
};

const COMMAND_CHECKS: [(&str, &[CheckId]); 2] = [
    ("lmx doctor", &checks::DOCTOR),
    ("lmx net check", &checks::NET),
];

#[derive(Serialize)]
struct Reference {
    commands: Vec<Entry>,
}

#[derive(Serialize)]
struct Entry {
    name: String,
    about: String,
    details: Option<String>,
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
    id: &'static str,
    what: &'static str,
}

fn render() -> String {
    let mut commands = Vec::new();
    walk(&Cli::command(), None, &mut commands);
    for (name, _) in COMMAND_CHECKS {
        assert!(
            commands.iter().any(|entry| entry.name == name),
            "no command {name}"
        );
    }
    let mut json = serde_json::to_string_pretty(&Reference { commands }).expect("a reference");
    json.push('\n');
    json
}

fn walk(command: &Command, parent: Option<&str>, commands: &mut Vec<Entry>) {
    let name = parent.map_or_else(
        || command.get_name().to_owned(),
        |parent| format!("{parent} {}", command.get_name()),
    );
    let checks = COMMAND_CHECKS
        .iter()
        .filter(|(listed, _)| *listed == name)
        .flat_map(|(_, checks)| checks.iter())
        .map(|check| Topic {
            id: check.name(),
            what: check.what(),
        })
        .collect();
    commands.push(Entry {
        name: name.clone(),
        about: text(command.get_about()),
        details: command.get_long_about().map(ToString::to_string),
        args: command
            .get_arguments()
            .filter(|arg| !arg.is_hide_set())
            .map(argument)
            .collect(),
        checks,
    });
    for subcommand in command.get_subcommands().filter(|sub| !sub.is_hide_set()) {
        walk(subcommand, Some(&name), commands);
    }
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
