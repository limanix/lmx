//! `lmx status --wait converged -g GENERATION`: waiting until an update has settled.
//!
//! After the host restarts the VM into a new generation, `lmxd` checks its health and removes the
//! older generations. The host waits for that here: `lmx` asks `lmxd` every two seconds until it
//! reports `Converged` and the booted generation is the one asked for, then answers with the full
//! status. A daemon that does not answer yet, as right after the restart, is waited for.

use std::{
    io,
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use lmx_facts::generations;
use lmx_model::{CONVERGED, Condition, DEGRADED, Envelope, ErrorBody, ErrorCode, Owner};
use serde_json::{Map, Value};

use crate::{
    output,
    owner::{self, CallError},
    status,
    system::System,
};

/// Longest wait unless `--timeout` says otherwise.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Pause between two questions to `lmxd`.
const POLL: Duration = Duration::from_secs(2);

/// Waits until `generation` is converged, at most `timeout`.
///
/// At the timeout, a `Degraded` generation is `system.degraded`, a daemon that never answered is
/// `owner.unavailable`, and anything else is `wait.timeout` with the last conditions.
pub(crate) fn converged(
    system: &System,
    generation: &str,
    timeout: Duration,
    json: bool,
) -> io::Result<ExitCode> {
    let socket = system.owner_socket();
    // A timeout beyond the clock's range is no deadline at all.
    let deadline = Instant::now().checked_add(timeout);
    let mut owner = None;
    let mut unanswered = CallError::Unavailable("lmxd did not answer".into());
    loop {
        match owner::status(&socket) {
            Ok(answer) => {
                let (generations, _) = generations::read(&system.generation_paths());
                if generations.booted.as_deref() == Some(generation) && holds(&answer, CONVERGED) {
                    let status = status::collect(system);
                    if json {
                        output::write_json(&Envelope::success(&status))?;
                    } else {
                        output::write_text(&status::render(&status))?;
                    }
                    return Ok(ExitCode::SUCCESS);
                }
                owner = Some(answer);
            }
            Err(error) => unanswered = error,
        }
        let left = deadline.map_or(POLL, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        });
        if left.is_zero() {
            break;
        }
        thread::sleep(POLL.min(left));
    }

    let Some(owner) = owner else {
        return owner::report(unanswered, json);
    };
    let error = match owner
        .conditions
        .iter()
        .find(|condition| condition.kind == DEGRADED)
    {
        Some(degraded) => failure(ErrorCode::SystemDegraded, &degraded.message, &owner),
        None => failure(
            ErrorCode::WaitTimeout,
            &format!(
                "Generation {generation} did not converge within {}.",
                span(timeout)
            ),
            &owner,
        ),
    };
    let status = output::failure(json, error, output::FAILURE)?;
    if !json {
        for condition in &owner.conditions {
            eprintln!("{}", condition.message);
        }
    }
    Ok(status)
}

/// Whether `owner` reports the condition `kind`.
fn holds(owner: &Owner, kind: &str) -> bool {
    owner
        .conditions
        .iter()
        .any(|condition| condition.kind == kind)
}

/// A failure with `code` and `message`, with the conditions `owner` reported last.
fn failure(code: ErrorCode, message: &str, owner: &Owner) -> ErrorBody {
    let mut details = Map::new();
    details.insert("conditions".into(), conditions(&owner.conditions));
    ErrorBody {
        code,
        message: message.to_owned(),
        details,
    }
}

/// `conditions` as a JSON array.
fn conditions(conditions: &[Condition]) -> Value {
    serde_json::to_value(conditions).unwrap_or(Value::Null)
}

/// `duration` for people, in whole minutes when it has no seconds.
fn span(duration: Duration) -> String {
    match duration.as_secs() {
        1 => "1 second".into(),
        60 => "1 minute".into(),
        seconds if seconds % 60 == 0 => format!("{} minutes", seconds / 60),
        seconds => format!("{seconds} seconds"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_wait_in_minutes_or_seconds() {
        assert_eq!(span(TIMEOUT), "10 minutes");
        assert_eq!(span(Duration::from_secs(60)), "1 minute");
        assert_eq!(span(Duration::from_secs(90)), "90 seconds");
    }

    #[test]
    fn a_timeout_carries_the_last_conditions() {
        let owner = Owner {
            version: "0.1.0".into(),
            conditions: vec![Condition {
                kind: "RestartRequired".into(),
                message: "Generation g2 is built; restart the VM to boot it.".into(),
            }],
            operations: vec![],
        };
        let error = failure(ErrorCode::WaitTimeout, "late", &owner);
        assert_eq!(
            Value::Object(error.details),
            serde_json::json!({"conditions": [{
                "type": "RestartRequired",
                "message": "Generation g2 is built; restart the VM to boot it."
            }]})
        );
    }
}
