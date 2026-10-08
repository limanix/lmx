//! `lmx status --wait converged -g GENERATION`: waiting until an update has settled.
//!
//! After the host restarts the VM into a new generation, `lmxd` checks its health and removes the
//! older generations. The host waits for that here: `lmx` asks `lmxd` every two seconds until it
//! reports `Converged` and the booted generation is the one asked for, then answers with the full
//! status. A daemon that does not answer yet, as right after the restart, is waited for. A failed
//! finalize ends the wait at once with `finalize.failed`: the generation works, and `lmxd` tries the
//! finalize again later.

use std::{
    io,
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use lmx_facts::generations;
use lmx_model::{
    CONVERGED, Condition, DEGRADED, Envelope, ErrorBody, ErrorCode, FINALIZE_FAILED, Owner,
};
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
/// A `FinalizeFailed` of the booted `generation` ends the wait at once with `finalize.failed`. At the
/// timeout, a `Degraded` generation is `system.degraded`, a daemon that never answered is
/// `owner.unavailable`, and anything else is `wait.timeout` with the last conditions.
pub(crate) fn converged(
    system: &System,
    generation: &str,
    timeout: Duration,
    json: bool,
) -> io::Result<ExitCode> {
    let socket = system.owner_socket();
    let deadline = Instant::now().checked_add(timeout);
    let mut owner = None;
    let mut unanswered = CallError::Unavailable("lmxd did not answer".into());
    loop {
        match owner::status(&socket) {
            Ok(answer) => {
                let (generations, _) = generations::read(&system.generation_paths());
                match verdict(&answer, generations.booted.as_deref(), generation) {
                    Verdict::Converged => {
                        let status = status::collect(system);
                        if json {
                            output::write_json(&Envelope::success(&status))?;
                        } else {
                            output::write_text(&status::render(&status))?;
                        }
                        return Ok(ExitCode::SUCCESS);
                    }
                    Verdict::FinalizeFailed(failed) => {
                        let error = failure(ErrorCode::FinalizeFailed, &failed.message, &answer);
                        return report(json, error, &answer);
                    }
                    Verdict::Wait => {}
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
    let error = match condition(&owner, DEGRADED) {
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
    report(json, error, &owner)
}

/// What one answer of `lmxd` means for a wait.
#[derive(Debug, PartialEq, Eq)]
enum Verdict<'a> {
    /// The generation is booted and converged: answer with the status.
    Converged,
    /// The generation is booted, but its finalize failed: answer `finalize.failed` at once, because
    /// the generation works.
    FinalizeFailed(&'a Condition),
    /// Ask again.
    Wait,
}

/// The verdict on `answer` for a wait for `generation` while `booted` is booted.
fn verdict<'a>(answer: &'a Owner, booted: Option<&str>, generation: &str) -> Verdict<'a> {
    if booted != Some(generation) {
        return Verdict::Wait;
    }
    if condition(answer, CONVERGED).is_some() {
        Verdict::Converged
    } else if let Some(failed) = condition(answer, FINALIZE_FAILED) {
        Verdict::FinalizeFailed(failed)
    } else {
        Verdict::Wait
    }
}

/// The condition `kind` that `owner` reports, if any.
fn condition<'a>(owner: &'a Owner, kind: &str) -> Option<&'a Condition> {
    owner
        .conditions
        .iter()
        .find(|condition| condition.kind == kind)
}

/// Writes `error`, and in text the other conditions `owner` reported, then gives the failure status.
fn report(json: bool, error: ErrorBody, owner: &Owner) -> io::Result<ExitCode> {
    let message = error.message.clone();
    let status = output::failure(json, error, output::FAILURE)?;
    if !json {
        for condition in owner
            .conditions
            .iter()
            .filter(|condition| condition.message != message)
        {
            eprintln!("{}", condition.message);
        }
    }
    Ok(status)
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
    fn a_failed_finalize_ends_the_wait_only_for_the_booted_generation() {
        let owner = |kind: &str| Owner {
            version: "0.0.2".into(),
            conditions: vec![Condition {
                kind: kind.into(),
                message: format!("{kind} message"),
            }],
            operations: vec![],
        };
        let converged = owner(CONVERGED);
        let failed = owner(FINALIZE_FAILED);
        assert_eq!(verdict(&converged, Some("g2"), "g2"), Verdict::Converged);
        assert_eq!(
            verdict(&failed, Some("g2"), "g2"),
            Verdict::FinalizeFailed(&failed.conditions[0])
        );
        assert_eq!(verdict(&failed, Some("g1"), "g2"), Verdict::Wait);
        assert_eq!(verdict(&converged, None, "g2"), Verdict::Wait);
        assert_eq!(verdict(&owner(DEGRADED), Some("g2"), "g2"), Verdict::Wait);
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
