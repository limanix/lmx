//! Versioned envelope of every `--json` answer.
//!
//! The LimaNix host reads one envelope from standard output per command:
//!
//! ```text
//! {"contract": 1, "ok": true,  "data": {…}}
//! {"contract": 1, "ok": false, "error": {"code": "disk.low", "message": "…", "details": {…}}}
//! ```
//!
//! `contract` lets the host refuse an answer it cannot decode instead of misreading it. `code` is for
//! programs and stays stable; `message` is for people and may change.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Host contract version written into every envelope.
pub const CONTRACT_VERSION: u32 = 1;

/// One `--json` answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope<T> {
    /// Contract version; always [`CONTRACT_VERSION`] when written by this crate.
    pub contract: u32,
    /// Whether the command succeeded; selects `data` or `error`.
    pub ok: bool,
    /// Result of a successful command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// Reason of a failed command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl<T> Envelope<T> {
    /// Wraps the result of a successful command.
    #[must_use]
    pub fn success(data: T) -> Self {
        Self {
            contract: CONTRACT_VERSION,
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    /// Wraps the reason of a failed command.
    #[must_use]
    pub fn failure(error: ErrorBody) -> Self {
        Self {
            contract: CONTRACT_VERSION,
            ok: false,
            data: None,
            error: Some(error),
        }
    }
}

/// Reason of a failed command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// Stable machine-readable code.
    pub code: ErrorCode,
    /// Explanation for people.
    pub message: String,
    /// Code-specific values, such as disk usage for [`ErrorCode::DiskLow`].
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub details: Map<String, Value>,
}

/// Stable failure codes of the host contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// `lmxd` is not reachable.
    #[serde(rename = "owner.unavailable")]
    OwnerUnavailable,
    /// `nixos-rebuild` failed for the requested generation.
    #[serde(rename = "apply.build_failed")]
    ApplyBuildFailed,
    /// The operation was cancelled by an explicit request.
    #[serde(rename = "apply.cancelled")]
    ApplyCancelled,
    /// Free bytes or inodes are below the platform minimum.
    #[serde(rename = "disk.low")]
    DiskLow,
    /// A required network destination, such as the binary cache, is unreachable.
    #[serde(rename = "network.unreachable")]
    NetworkUnreachable,
    /// The caller is not allowed to run the operation.
    #[serde(rename = "permission.denied")]
    PermissionDenied,
    /// The mounted inputs belong to a different generation than requested.
    #[serde(rename = "generation.mismatch")]
    GenerationMismatch,
    /// The caller requested a contract version this binary does not speak.
    #[serde(rename = "contract.unsupported")]
    ContractUnsupported,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_carries_data_without_error() {
        let json = serde_json::to_value(Envelope::success(7)).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({"contract": 1, "ok": true, "data": 7})
        );
    }

    #[test]
    fn failure_carries_a_stable_code() {
        let envelope = Envelope::<()>::failure(ErrorBody {
            code: ErrorCode::DiskLow,
            message: "less than 10% of the guest disk is free".into(),
            details: Map::new(),
        });
        let json = serde_json::to_value(envelope).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({
                "contract": 1,
                "ok": false,
                "error": {"code": "disk.low", "message": "less than 10% of the guest disk is free"}
            })
        );
    }

    #[test]
    fn failure_decodes_without_data() {
        /// A result without a meaningful default, like most command results.
        #[derive(Debug, PartialEq, Deserialize)]
        struct Reserve {
            /// Bytes freed by the collection.
            freed_bytes: u64,
        }

        let envelope: Envelope<Reserve> = serde_json::from_str(
            r#"{"contract": 1, "ok": false, "error": {"code": "disk.low", "message": "full"}}"#,
        )
        .expect("a failure decodes");
        assert_eq!(envelope.data, None);
        assert_eq!(
            envelope.error.map(|error| error.code),
            Some(ErrorCode::DiskLow)
        );
    }

    #[test]
    fn error_codes_keep_their_wire_names() {
        for (code, name) in [
            (ErrorCode::OwnerUnavailable, "owner.unavailable"),
            (ErrorCode::ApplyBuildFailed, "apply.build_failed"),
            (ErrorCode::ApplyCancelled, "apply.cancelled"),
            (ErrorCode::DiskLow, "disk.low"),
            (ErrorCode::NetworkUnreachable, "network.unreachable"),
            (ErrorCode::PermissionDenied, "permission.denied"),
            (ErrorCode::GenerationMismatch, "generation.mismatch"),
            (ErrorCode::ContractUnsupported, "contract.unsupported"),
        ] {
            assert_eq!(serde_json::to_value(code).expect("serialize"), name);
            assert_eq!(
                serde_json::from_value::<ErrorCode>(name.into()).expect("deserialize"),
                code
            );
        }
    }
}
