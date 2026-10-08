//! `lmx status`: what the guest is right now.
//!
//! Every fact is read independently. A fact that cannot be read becomes `null` with a [`Problem`],
//! and the host and people still get the rest. The owner part comes from `lmxd` when it answers
//! within two seconds. `--short` asks `lmxd` only, and prints what needs attention.

use std::{io, process::ExitCode, time::Duration};

use lmx_facts::{FactError, disk, generations, network, units};
use lmx_model::{
    DEGRADED, DISK_LOW, Envelope, FINALIZE_FAILED, OUT_OF_DATE, Owner, Problem, RESTART_REQUIRED,
    Status,
};

use crate::{
    cli::{Goal, StatusArgs},
    format, layout, output, owner,
    system::System,
    wait,
};

/// How long `lmx status --short` waits for `lmxd`; a prompt must never hang.
const SHORT_TIMEOUT: Duration = Duration::from_millis(200);

/// Runs `lmx status`, waits for a goal first with `--wait`, or prints the short form.
pub(crate) fn run(system: &System, args: &StatusArgs) -> io::Result<ExitCode> {
    if args.short {
        let words = owner::status_within(&system.owner_socket(), SHORT_TIMEOUT)
            .map_or_else(|_| vec!["lmxd?"], |owner| short(&owner));
        if !words.is_empty() {
            output::write_text(&format!("{}\n", words.join(" ")))?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    if let (Some(Goal::Converged), Some(generation)) = (args.wait, &args.generation) {
        let timeout = args.timeout.unwrap_or(wait::TIMEOUT);
        return wait::converged(system, generation, timeout, args.output.json);
    }
    let status = collect(system);
    if args.output.json {
        output::write_json(&Envelope::success(status))?;
    } else {
        output::write_text(&render(&status))?;
    }
    Ok(ExitCode::SUCCESS)
}

/// The words of `lmx status --short`: what needs attention, most urgent first; none when all is well.
fn short(owner: &Owner) -> Vec<&'static str> {
    let holds = |kind: &str| {
        owner
            .conditions
            .iter()
            .any(|condition| condition.kind == kind)
    };
    let applying = owner
        .operations
        .iter()
        .any(|operation| operation.kind == "SystemApply");
    let mut words = Vec::new();
    if holds(DEGRADED) {
        words.push("degraded");
    }
    if holds(FINALIZE_FAILED) {
        words.push("finalize");
    }
    if holds(DISK_LOW) {
        words.push("disk-low");
    }
    if holds(RESTART_REQUIRED) {
        words.push("restart");
    }
    if applying {
        words.push("applying");
    } else if holds(OUT_OF_DATE) {
        words.push("apply");
    }
    words
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
        owner: owner::status(&system.owner_socket())
            .map_err(|error| {
                problems.push(Problem {
                    fact: "owner".into(),
                    message: error.to_string(),
                });
            })
            .ok(),
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
    let mut rows = vec![
        (
            "Generation",
            format!(
                "desired {}, built {}, booted {}",
                generations.desired.clone().unwrap_or_else(unknown),
                generations.built.clone().unwrap_or_else(unknown),
                generations.booted.clone().unwrap_or_else(unknown),
            ),
        ),
        (
            "Disk",
            status.disk.as_ref().map_or_else(unknown, format::disk),
        ),
        (
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
        ),
        (
            "Failed units",
            status.failed_units.as_ref().map_or_else(unknown, |units| {
                if units.is_empty() {
                    "none".into()
                } else {
                    units.join(", ")
                }
            }),
        ),
    ];
    rows.push((
        "Owner",
        status.owner.as_ref().map_or_else(unknown, |owner| {
            let doing: Vec<String> = owner
                .operations
                .iter()
                .map(|operation| format!("{} {}", operation.kind, operation.phase))
                .collect();
            let doing = if doing.is_empty() {
                "idle".to_owned()
            } else {
                doing.join(", ")
            };
            format!("lmxd {}, {doing}", owner.version)
        }),
    ));
    for condition in status.owner.iter().flat_map(|owner| &owner.conditions) {
        rows.push(("Condition", condition.message.clone()));
    }
    for problem in &status.problems {
        rows.push(("Problem", format!("{}: {}", problem.fact, problem.message)));
    }
    layout::rows(&rows, LABEL)
}

#[cfg(test)]
mod tests {
    use lmx_model::{Condition, DiskUsage, Generations, Interface, Operation, Owner};

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
            owner: Some(Owner {
                version: "0.1.0".into(),
                conditions: vec![Condition {
                    kind: "DiskLow".into(),
                    message: "Less than 10% of the guest disk is free.".into(),
                }],
                operations: vec![Operation {
                    task: "store-collect-1".into(),
                    kind: "StoreCollect".into(),
                    phase: "running".into(),
                    created_at: 0,
                }],
            }),
            problems: vec![],
        };
        assert_eq!(
            render(&status),
            "Generation    desired 0123456789ab, built 0123456789ab, booted unknown\n\
             Disk          8.0 GiB of 16 GiB free, 495616 of 1048576 inodes free\n\
             Network       enp0s1 192.0.2.10\n\
             Failed units  none\n\
             Owner         lmxd 0.1.0, StoreCollect running\n\
             Condition     Less than 10% of the guest disk is free.\n"
        );
    }

    #[test]
    fn the_short_form_names_only_what_needs_attention() {
        let owner = |kinds: &[&str], applying: bool| Owner {
            version: "0.1.0".into(),
            conditions: kinds
                .iter()
                .map(|kind| Condition {
                    kind: (*kind).into(),
                    message: String::new(),
                })
                .collect(),
            operations: applying
                .then(|| Operation {
                    task: "system-apply-1".into(),
                    kind: "SystemApply".into(),
                    phase: "running".into(),
                    created_at: 0,
                })
                .into_iter()
                .collect(),
        };
        assert!(short(&owner(&["Converged"], false)).is_empty());
        assert_eq!(
            short(&owner(&[RESTART_REQUIRED, DISK_LOW, DEGRADED], false)),
            ["degraded", "disk-low", "restart"]
        );
        assert_eq!(short(&owner(&[OUT_OF_DATE], false)), ["apply"]);
        assert_eq!(short(&owner(&[OUT_OF_DATE], true)), ["applying"]);
        assert_eq!(short(&owner(&[FINALIZE_FAILED], false)), ["finalize"]);
    }

    #[test]
    fn leaves_out_inodes_of_a_file_system_without_an_inode_table() {
        let status = Status {
            disk: Some(DiskUsage {
                bytes: 16 << 30,
                free_bytes: 9 << 30,
                available_bytes: 8 << 30,
                inodes: 0,
                free_inodes: 0,
            }),
            ..Status::default()
        };
        assert!(
            render(&status).contains("\nDisk          8.0 GiB of 16 GiB free\n"),
            "{}",
            render(&status)
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
        let problem: Vec<&str> = text.lines().skip(5).collect();
        assert_eq!(
            problem,
            [
                "Problem       interfaces: ip failed (exit status: 1): invalid option",
                "              Usage: ip OBJECT",
            ]
        );
    }
}
