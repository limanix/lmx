//! `lmx status`: what the guest is right now.
//!
//! Every fact is read independently. A fact that cannot be read becomes `null` with a
//! [`Problem`], so the host and people always get the rest.

use std::{io, process::ExitCode};

use lmx_facts::{FactError, disk, generations, network, units};
use lmx_model::{Envelope, Problem, Status};

use crate::{cli::OutputArgs, format::gibibytes, output, system::System};

/// Runs `lmx status`.
pub(crate) fn run(system: &System, args: &OutputArgs) -> io::Result<ExitCode> {
    let status = collect(system);
    if args.json {
        output::write_json(&Envelope::success(status))?;
    } else {
        output::write_text(&render(&status))?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Reads every fact of [`Status`].
pub(crate) fn collect(system: &System) -> Status {
    let mut problems = Vec::new();
    if let Err(message) = &system.config {
        problems.push(Problem {
            fact: "config".into(),
            message: message.clone(),
        });
    }

    let (generations, unreadable) = generations::read(&system.generation_paths());
    problems.extend(unreadable.into_iter().map(|error| Problem {
        fact: "generations".into(),
        message: error.to_string(),
    }));

    Status {
        generations,
        disk: record("disk", disk::usage(&system.store()), &mut problems),
        interfaces: record(
            "interfaces",
            network::interfaces(&system.ip()),
            &mut problems,
        ),
        failed_units: record(
            "failed_units",
            units::failed(&system.systemctl()),
            &mut problems,
        ),
        problems,
    }
}

/// Keeps a fact, or records why it is missing.
fn record<T>(fact: &str, result: Result<T, FactError>, problems: &mut Vec<Problem>) -> Option<T> {
    result
        .map_err(|error| {
            problems.push(Problem {
                fact: fact.into(),
                message: error.to_string(),
            });
        })
        .ok()
}

/// Width of the label column in text output.
const LABEL: usize = 14;

/// Renders status as aligned text for people.
pub(crate) fn render(status: &Status) -> String {
    let unknown = || "unknown".to_owned();
    let generations = &status.generations;
    let mut text = String::new();
    // Continuation lines, such as a tool's multi-line standard error, stay in the value column.
    let mut row = |label: &str, value: String| {
        let mut lines = value.lines();
        let first = lines.next().unwrap_or_default();
        text.push_str(&format!("{label:<LABEL$}{first}\n"));
        for line in lines {
            text.push_str(&format!("{:LABEL$}{line}\n", ""));
        }
    };

    row(
        "Generation",
        format!(
            "desired {}, built {}, booted {}",
            generations.desired.clone().unwrap_or_else(unknown),
            generations.built.clone().unwrap_or_else(unknown),
            generations.booted.clone().unwrap_or_else(unknown),
        ),
    );
    row(
        "Disk",
        status.disk.map_or_else(unknown, |disk| {
            format!(
                "{} of {} free, {} of {} inodes free",
                gibibytes(disk.free_bytes),
                gibibytes(disk.bytes),
                disk.free_inodes,
                disk.inodes
            )
        }),
    );
    row(
        "Network",
        status
            .interfaces
            .as_ref()
            .map_or_else(unknown, |interfaces| {
                let addressed: Vec<String> = interfaces
                    .iter()
                    .filter(|interface| !interface.ipv4.is_empty())
                    .map(|interface| format!("{} {}", interface.name, interface.ipv4.join(" ")))
                    .collect();
                if addressed.is_empty() {
                    "no global IPv4 address".into()
                } else {
                    addressed.join(", ")
                }
            }),
    );
    row(
        "Failed units",
        status.failed_units.as_ref().map_or_else(unknown, |units| {
            if units.is_empty() {
                "none".into()
            } else {
                units.join(", ")
            }
        }),
    );
    for problem in &status.problems {
        row("Problem", format!("{}: {}", problem.fact, problem.message));
    }
    text
}

#[cfg(test)]
mod tests {
    use lmx_model::{DiskUsage, Generations, Interface};

    use super::*;

    #[test]
    fn renders_every_fact_on_its_own_line() {
        let status = Status {
            generations: Generations {
                desired: Some("0123456789ab".into()),
                built: Some("0123456789ab".into()),
                booted: None,
            },
            disk: Some(DiskUsage {
                bytes: 16 << 30,
                free_bytes: 9 << 30,
                available_bytes: 8 << 30,
                inodes: 1_048_576,
                free_inodes: 495_616,
            }),
            interfaces: Some(vec![
                Interface {
                    name: "lo".into(),
                    mac: None,
                    ipv4: vec![],
                },
                Interface {
                    name: "enp0s1".into(),
                    mac: None,
                    ipv4: vec!["192.0.2.10".into()],
                },
            ]),
            failed_units: Some(vec![]),
            problems: vec![],
        };
        assert_eq!(
            render(&status),
            "Generation    desired 0123456789ab, built 0123456789ab, booted unknown\n\
             Disk          9.0 GiB of 16 GiB free, 495616 of 1048576 inodes free\n\
             Network       enp0s1 192.0.2.10\n\
             Failed units  none\n"
        );
    }

    #[test]
    fn keeps_continuation_lines_of_a_problem_in_the_value_column() {
        let status = Status {
            problems: vec![Problem {
                fact: "interfaces".into(),
                message: "ip failed (exit status: 1): invalid option\nUsage: ip OBJECT".into(),
            }],
            ..Status::default()
        };
        let text = render(&status);
        let problem: Vec<&str> = text.lines().skip(4).collect();
        assert_eq!(
            problem,
            [
                "Problem       interfaces: ip failed (exit status: 1): invalid option",
                "              Usage: ip OBJECT",
            ]
        );
    }
}
