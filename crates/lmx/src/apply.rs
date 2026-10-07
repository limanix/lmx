//! `lmx apply`: building the generation the host mounted for the next boot.
//!
//! The apply runs in `lmxd`, which keeps going when this command is interrupted; running it again
//! attaches to the running apply. `--follow` streams its progress: with `--json` as JSON Lines that
//! end with the contract envelope, otherwise the build's output and a closing sentence.
//! `lmx apply cancel` stops it.

use std::{
    io::{self, Write},
    process::ExitCode,
};

use lmx_model::{
    Apply, ApplyEvent, ApplyPhase, ApplyState, CancelApply, Envelope, ErrorCode, OutputStream,
};

use crate::{output, owner, system::System};

/// Exit status of a cancelled apply, as of a command stopped with Ctrl-C.
const CANCELLED: u8 = 130;

/// Runs `lmx apply -g GENERATION`.
pub(crate) fn run(
    system: &System,
    generation: &str,
    follow: bool,
    json: bool,
) -> io::Result<ExitCode> {
    let answer = owner::apply(&system.owner_socket(), generation, follow, |event| {
        if json {
            output::write_json_line(&event)
        } else {
            write_event(&event)
        }
    });
    match answer {
        Ok(Ok(apply)) => {
            if json {
                output::write_json(&Envelope::success(&apply))?;
            } else {
                output::write_text(&sentence(&apply))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Ok(Err(error)) => {
            let status = match error.code {
                ErrorCode::ApplyCancelled => CANCELLED,
                ErrorCode::OwnerUnavailable => output::UNAVAILABLE,
                _ => output::FAILURE,
            };
            output::failure(json, error, status)
        }
        Err(error) => owner::report(error, json),
    }
}

/// Runs `lmx apply cancel -g GENERATION`.
pub(crate) fn cancel(system: &System, generation: &str, json: bool) -> io::Result<ExitCode> {
    match owner::cancel_apply(&system.owner_socket(), generation) {
        Ok(Ok(cancel)) => {
            if json {
                output::write_json(&Envelope::success(cancel))?;
            } else {
                output::write_text(&cancelled(generation, cancel))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Ok(Err(error)) => output::failure(json, error, output::FAILURE),
        Err(error) => owner::report(error, json),
    }
}

/// Shows an event to people: the build's lines on their own streams, the rest on standard error.
fn write_event(event: &ApplyEvent) -> io::Result<()> {
    match event {
        ApplyEvent::Output {
            stream: OutputStream::Stdout,
            line,
            ..
        } => output::write_text(&format!("{line}\n")),
        ApplyEvent::Output { line, .. } => write_stderr(line),
        ApplyEvent::Phase { phase } => write_stderr(match phase {
            ApplyPhase::Environment => "Installing the environment files.",
            ApplyPhase::Reserve => "Making room in the store.",
            ApplyPhase::Build => "Building the generation for the next boot.",
        }),
        ApplyEvent::Warning { message, .. } => write_stderr(&format!("Warning: {message}")),
        ApplyEvent::Lagged { skipped } => {
            write_stderr(&format!("lmx: {skipped} lines of the apply were skipped."))
        }
    }
}

/// Writes one line to standard error.
fn write_stderr(line: &str) -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    writeln!(stderr, "{line}")
}

/// The closing sentence of an apply for people.
fn sentence(apply: &Apply) -> String {
    let generation = &apply.generation;
    match apply.state {
        ApplyState::RestartRequired => {
            format!("Generation {generation} is built; restart the VM to boot it.\n")
        }
        ApplyState::Running => format!(
            "lmxd is applying generation {generation}; follow it with lmx apply -g {generation} \
             --follow.\n"
        ),
    }
}

/// The answer of a cancel for people.
fn cancelled(generation: &str, cancel: CancelApply) -> String {
    if cancel.cancelled {
        format!("Cancelled the apply of generation {generation}.\n")
    } else {
        format!("No apply of generation {generation} is running.\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_people_what_to_do_next() {
        let built = Apply {
            generation: "g1".into(),
            state: ApplyState::RestartRequired,
        };
        assert_eq!(
            sentence(&built),
            "Generation g1 is built; restart the VM to boot it.\n"
        );
        let running = Apply {
            state: ApplyState::Running,
            ..built
        };
        assert_eq!(
            sentence(&running),
            "lmxd is applying generation g1; follow it with lmx apply -g g1 --follow.\n"
        );
        assert_eq!(
            cancelled("g1", CancelApply { cancelled: false }),
            "No apply of generation g1 is running.\n"
        );
    }
}
