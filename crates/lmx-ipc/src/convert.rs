//! Conversions between the `lmx.v1` messages and the `lmx-model` contract types.

use lmx_model::{Condition, DiskUsage, ErrorBody, ErrorCode, Operation, Owner, Reserve};
use serde_json::{Map, Value};

use crate::proto;

impl From<DiskUsage> for proto::DiskUsage {
    fn from(usage: DiskUsage) -> Self {
        Self {
            bytes: usage.bytes,
            free_bytes: usage.free_bytes,
            available_bytes: usage.available_bytes,
            inodes: usage.inodes,
            free_inodes: usage.free_inodes,
        }
    }
}

impl From<proto::DiskUsage> for DiskUsage {
    fn from(usage: proto::DiskUsage) -> Self {
        Self {
            bytes: usage.bytes,
            free_bytes: usage.free_bytes,
            available_bytes: usage.available_bytes,
            inodes: usage.inodes,
            free_inodes: usage.free_inodes,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Usage with distinct values in every field.
    const USAGE: DiskUsage = DiskUsage {
        bytes: 1,
        free_bytes: 2,
        available_bytes: 3,
        inodes: 4,
        free_inodes: 5,
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
    fn rejects_an_answer_without_an_outcome() {
        let response = proto::ReserveResponse { outcome: None };
        assert_eq!(
            reserve_outcome(response).map_err(|error| error.to_string()),
            Err("lmxd answered without an outcome".into())
        );
    }
}
