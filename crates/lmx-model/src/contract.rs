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

use serde::{Deserialize, Deserializer, Serialize, Serializer};
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
///
/// A code this binary does not know, such as one from a newer `lmxd`, is kept as
/// [`ErrorCode::Other`], so it can be passed on unchanged and read as a generic failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// `lmxd` is not reachable.
    OwnerUnavailable,
    /// `nixos-rebuild` failed for the requested generation.
    ApplyBuildFailed,
    /// The operation was cancelled by an explicit request.
    ApplyCancelled,
    /// The environment files of the generation could not be installed.
    ApplyEnvironmentFailed,
    /// Free bytes or inodes are below the platform minimum.
    DiskLow,
    /// The usage of the store file system cannot be read.
    DiskUnreadable,
    /// A required network destination, such as the binary cache, is unreachable.
    NetworkUnreachable,
    /// The caller is not allowed to run the operation.
    PermissionDenied,
    /// The mounted inputs belong to a different generation than requested.
    GenerationMismatch,
    /// The caller requested a contract version this binary does not speak.
    ContractUnsupported,
    /// The booted generation failed its health check.
    SystemDegraded,
    /// A wait ended before its condition held.
    WaitTimeout,
    /// A code this binary does not know, as received.
    Other(String),
}

impl ErrorCode {
    /// Wire name of the code, such as `disk.low`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::OwnerUnavailable => "owner.unavailable",
            Self::ApplyBuildFailed => "apply.build_failed",
            Self::ApplyCancelled => "apply.cancelled",
            Self::ApplyEnvironmentFailed => "apply.environment_failed",
            Self::DiskLow => "disk.low",
            Self::DiskUnreadable => "disk.unreadable",
            Self::NetworkUnreachable => "network.unreachable",
            Self::PermissionDenied => "permission.denied",
            Self::GenerationMismatch => "generation.mismatch",
            Self::ContractUnsupported => "contract.unsupported",
            Self::SystemDegraded => "system.degraded",
            Self::WaitTimeout => "wait.timeout",
            Self::Other(code) => code,
        }
    }

    /// Code with the wire name `name`, or [`ErrorCode::Other`] for a name this binary does not know.
    #[must_use]
    pub fn from_wire(name: &str) -> Self {
        match name {
            "owner.unavailable" => Self::OwnerUnavailable,
            "apply.build_failed" => Self::ApplyBuildFailed,
            "apply.cancelled" => Self::ApplyCancelled,
            "apply.environment_failed" => Self::ApplyEnvironmentFailed,
            "disk.low" => Self::DiskLow,
            "disk.unreadable" => Self::DiskUnreadable,
            "network.unreachable" => Self::NetworkUnreachable,
            "permission.denied" => Self::PermissionDenied,
            "generation.mismatch" => Self::GenerationMismatch,
            "contract.unsupported" => Self::ContractUnsupported,
            "system.degraded" => Self::SystemDegraded,
            "wait.timeout" => Self::WaitTimeout,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|name| Self::from_wire(&name))
    }
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
            (
                ErrorCode::ApplyEnvironmentFailed,
                "apply.environment_failed",
            ),
            (ErrorCode::DiskLow, "disk.low"),
            (ErrorCode::DiskUnreadable, "disk.unreadable"),
            (ErrorCode::NetworkUnreachable, "network.unreachable"),
            (ErrorCode::PermissionDenied, "permission.denied"),
            (ErrorCode::GenerationMismatch, "generation.mismatch"),
            (ErrorCode::ContractUnsupported, "contract.unsupported"),
            (ErrorCode::SystemDegraded, "system.degraded"),
            (ErrorCode::WaitTimeout, "wait.timeout"),
        ] {
            assert_eq!(serde_json::to_value(&code).expect("serialize"), name);
            assert_eq!(
                serde_json::from_value::<ErrorCode>(name.into()).expect("deserialize"),
                code
            );
        }
    }

    #[test]
    fn unknown_codes_pass_through_unchanged() {
        let code: ErrorCode = serde_json::from_value("store.busy".into()).expect("deserialize");
        assert_eq!(code, ErrorCode::Other("store.busy".into()));
        assert_eq!(
            serde_json::to_value(&code).expect("serialize"),
            "store.busy"
        );
    }
}
