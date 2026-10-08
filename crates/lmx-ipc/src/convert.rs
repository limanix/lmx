//! Conversions between the `lmx.v1` messages and the `lmx-model` contract types.

use lmx_model::{
    Apply, ApplyEvent, ApplyPhase, ApplyState, CancelApply, Condition, DiskUsage, ErrorBody,
    ErrorCode, Operation, OutputStream, Owner, Reserve,
};
use serde_json::{Map, Value};

use crate::proto;

impl From<DiskUsage> for proto::DiskUsage {
    fn from(usage: DiskUsage) -> Self {
        Self {
            available_bytes: usage.available_bytes,
            free_inodes: usage.free_inodes,
            free_bytes: usage.free_bytes,
            inodes: usage.inodes,
            bytes: usage.bytes,
        }
    }
}

impl From<proto::DiskUsage> for DiskUsage {
    fn from(usage: proto::DiskUsage) -> Self {
        Self {
            available_bytes: usage.available_bytes,
            free_inodes: usage.free_inodes,
            free_bytes: usage.free_bytes,
            inodes: usage.inodes,
            bytes: usage.bytes,
        }
    }
}

impl From<Owner> for proto::StatusResponse {
    fn from(owner: Owner) -> Self {
        Self {
            version: owner.version,
            conditions: owner
                .conditions
                .into_iter()
                .map(|condition| proto::Condition {
                    r#type: condition.kind,
                    message: condition.message,
                })
                .collect(),
            operations: owner
                .operations
                .into_iter()
                .map(|operation| proto::Operation {
                    task: operation.task,
                    kind: operation.kind,
                    phase: operation.phase,
                    created_at: operation.created_at,
                })
                .collect(),
        }
    }
}

impl From<proto::StatusResponse> for Owner {
    fn from(response: proto::StatusResponse) -> Self {
        Self {
            version: response.version,
            conditions: response
                .conditions
                .into_iter()
                .map(|condition| Condition {
                    kind: condition.r#type,
                    message: condition.message,
                })
                .collect(),
            operations: response
                .operations
                .into_iter()
                .map(|operation| Operation {
                    task: operation.task,
                    kind: operation.kind,
                    phase: operation.phase,
                    created_at: operation.created_at,
                })
                .collect(),
        }
    }
}

impl From<Reserve> for proto::ReserveResult {
    fn from(reserve: Reserve) -> Self {
        Self {
            before: Some(reserve.before.into()),
            after: Some(reserve.after.into()),
            freed_bytes: reserve.freed_bytes,
            collected: reserve.collected,
        }
    }
}

impl From<ErrorBody> for proto::Failure {
    fn from(error: ErrorBody) -> Self {
        Self {
            code: error.code.as_str().to_owned(),
            message: error.message,
            details: if error.details.is_empty() {
                String::new()
            } else {
                Value::Object(error.details).to_string()
            },
        }
    }
}

impl From<proto::Failure> for ErrorBody {
    fn from(failure: proto::Failure) -> Self {
        let details = match serde_json::from_str(&failure.details) {
            Ok(Value::Object(details)) => details,
            _ => Map::new(),
        };
        Self {
            code: ErrorCode::from_wire(&failure.code),
            message: failure.message,
            details,
        }
    }
}

/// An answer of `lmxd` that breaks the `lmx.v1` protocol, such as one without an outcome.
#[derive(Debug, PartialEq, Eq)]
pub struct InvalidAnswer(pub &'static str);

impl std::fmt::Display for InvalidAnswer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "lmxd answered without {}", self.0)
    }
}

impl std::error::Error for InvalidAnswer {}

/// Outcome of a `Reserve` call: the result, or the failure the contract names.
pub fn reserve_outcome(
    response: proto::ReserveResponse,
) -> Result<Result<Reserve, ErrorBody>, InvalidAnswer> {
    match response.outcome {
        Some(proto::reserve_response::Outcome::Result(result)) => Ok(Ok(Reserve {
            before: result.before.ok_or(InvalidAnswer("usage before"))?.into(),
            after: result.after.ok_or(InvalidAnswer("usage after"))?.into(),
            freed_bytes: result.freed_bytes,
            collected: result.collected,
        })),
        Some(proto::reserve_response::Outcome::Failure(failure)) => Ok(Err(failure.into())),
        None => Err(InvalidAnswer("an outcome")),
    }
}

/// Wire name of an apply phase.
const fn phase_name(phase: ApplyPhase) -> &'static str {
    match phase {
        ApplyPhase::Environment => "environment",
        ApplyPhase::Reserve => "reserve",
        ApplyPhase::Build => "build",
    }
}

/// Apply phase with the wire name `name`.
fn phase(name: &str) -> Option<ApplyPhase> {
    [
        ApplyPhase::Environment,
        ApplyPhase::Reserve,
        ApplyPhase::Build,
    ]
    .into_iter()
    .find(|phase| phase_name(*phase) == name)
}

/// Wire name of an apply state.
const fn state_name(state: ApplyState) -> &'static str {
    match state {
        ApplyState::Running => "running",
        ApplyState::RestartRequired => "restart_required",
    }
}

/// Apply state with the wire name `name`.
fn state(name: &str) -> Option<ApplyState> {
    [ApplyState::Running, ApplyState::RestartRequired]
        .into_iter()
        .find(|state| state_name(*state) == name)
}

impl From<ApplyEvent> for proto::ApplyEvent {
    fn from(event: ApplyEvent) -> Self {
        use proto::apply_event::Event;
        let event = match event {
            ApplyEvent::Phase { phase } => Event::Phase(phase_name(phase).to_owned()),
            ApplyEvent::Output {
                stream,
                line,
                truncated,
            } => Event::Output(proto::OutputLine {
                stderr: stream == OutputStream::Stderr,
                line: line.into_bytes(),
                truncated,
            }),
            ApplyEvent::Warning { code, message } => Event::Warning(
                ErrorBody {
                    code,
                    message,
                    details: Map::new(),
                }
                .into(),
            ),
            ApplyEvent::Lagged { skipped } => Event::Lagged(skipped),
        };
        Self { event: Some(event) }
    }
}

/// The last event of an apply: how it ended, or that it runs.
pub fn outcome_event(outcome: Result<Apply, ErrorBody>) -> proto::ApplyEvent {
    let outcome = match outcome {
        Ok(apply) => proto::apply_outcome::Outcome::Result(proto::ApplyResult {
            generation: apply.generation,
            state: state_name(apply.state).to_owned(),
        }),
        Err(error) => proto::apply_outcome::Outcome::Failure(error.into()),
    };
    proto::ApplyEvent {
        event: Some(proto::apply_event::Event::Outcome(proto::ApplyOutcome {
            outcome: Some(outcome),
        })),
    }
}

/// One `Apply` event as the client reads it.
#[derive(Debug, PartialEq, Eq)]
pub enum ApplyMessage {
    /// Progress of the apply.
    Event(ApplyEvent),
    /// How the apply ended, or that it runs; nothing follows.
    Outcome(Result<Apply, ErrorBody>),
}

/// Reads one `Apply` event.
pub fn apply_message(event: proto::ApplyEvent) -> Result<ApplyMessage, InvalidAnswer> {
    use proto::apply_event::Event;
    Ok(match event.event.ok_or(InvalidAnswer("an apply event"))? {
        Event::Phase(name) => ApplyMessage::Event(ApplyEvent::Phase {
            phase: phase(&name).ok_or(InvalidAnswer("a known apply phase"))?,
        }),
        Event::Output(output) => ApplyMessage::Event(ApplyEvent::Output {
            stream: if output.stderr {
                OutputStream::Stderr
            } else {
                OutputStream::Stdout
            },
            line: String::from_utf8_lossy(&output.line).into_owned(),
            truncated: output.truncated,
        }),
        Event::Warning(failure) => {
            let warning = ErrorBody::from(failure);
            ApplyMessage::Event(ApplyEvent::Warning {
                code: warning.code,
                message: warning.message,
            })
        }
        Event::Lagged(skipped) => ApplyMessage::Event(ApplyEvent::Lagged { skipped }),
        Event::Outcome(outcome) => {
            ApplyMessage::Outcome(match outcome.outcome.ok_or(InvalidAnswer("an outcome"))? {
                proto::apply_outcome::Outcome::Result(result) => Ok(Apply {
                    state: state(&result.state).ok_or(InvalidAnswer("a known apply state"))?,
                    generation: result.generation,
                }),
                proto::apply_outcome::Outcome::Failure(failure) => Err(failure.into()),
            })
        }
    })
}

/// Outcome of a `CancelApply` call: the answer, or the failure the contract names.
pub fn cancel_outcome(
    response: proto::CancelApplyResponse,
) -> Result<Result<CancelApply, ErrorBody>, InvalidAnswer> {
    match response.outcome {
        Some(proto::cancel_apply_response::Outcome::Cancelled(cancelled)) => {
            Ok(Ok(CancelApply { cancelled }))
        }
        Some(proto::cancel_apply_response::Outcome::Failure(failure)) => Ok(Err(failure.into())),
        None => Err(InvalidAnswer("an outcome")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USAGE: DiskUsage = DiskUsage {
        available_bytes: 3,
        free_inodes: 5,
        free_bytes: 2,
        inodes: 4,
        bytes: 1,
    };

    #[test]
    fn owner_state_survives_the_wire() {
        let owner = Owner {
            version: "0.1.0".into(),
            conditions: vec![Condition {
                kind: "DiskLow".into(),
                message: "Less than 10% of the guest disk is free.".into(),
            }],
            operations: vec![Operation {
                task: "store-collect-1".into(),
                kind: "StoreCollect".into(),
                phase: "running".into(),
                created_at: 1_791_374_400_000,
            }],
        };
        assert_eq!(
            Owner::from(proto::StatusResponse::from(owner.clone())),
            owner
        );
    }

    #[test]
    fn reserve_results_and_failures_survive_the_wire() {
        let reserve = Reserve {
            before: USAGE,
            after: USAGE,
            freed_bytes: 7,
            collected: true,
        };
        let response = proto::ReserveResponse {
            outcome: Some(proto::reserve_response::Outcome::Result(
                reserve.clone().into(),
            )),
        };
        assert_eq!(reserve_outcome(response), Ok(Ok(reserve)));

        let mut details = Map::new();
        details.insert("freed_bytes".into(), 7.into());
        let error = ErrorBody {
            code: ErrorCode::DiskLow,
            message: "full".into(),
            details,
        };
        let response = proto::ReserveResponse {
            outcome: Some(proto::reserve_response::Outcome::Failure(
                error.clone().into(),
            )),
        };
        assert_eq!(reserve_outcome(response), Ok(Err(error)));
    }

    #[test]
    fn apply_events_and_outcomes_survive_the_wire() {
        for event in [
            ApplyEvent::Phase {
                phase: ApplyPhase::Build,
            },
            ApplyEvent::Output {
                stream: OutputStream::Stderr,
                line: "building the system configuration...".into(),
                truncated: true,
            },
            ApplyEvent::Warning {
                code: ErrorCode::DiskLow,
                message: "full".into(),
            },
            ApplyEvent::Lagged { skipped: 3 },
        ] {
            assert_eq!(
                apply_message(event.clone().into()),
                Ok(ApplyMessage::Event(event))
            );
        }
        let applied = Apply {
            generation: "0123456789ab".into(),
            state: ApplyState::RestartRequired,
        };
        assert_eq!(
            apply_message(outcome_event(Ok(applied.clone()))),
            Ok(ApplyMessage::Outcome(Ok(applied)))
        );
    }

    #[test]
    fn rejects_an_answer_without_an_outcome() {
        let response = proto::ReserveResponse { outcome: None };
        assert_eq!(
            reserve_outcome(response).map_err(|error| error.to_string()),
            Err("lmxd answered without an outcome".into())
        );
    }
}
