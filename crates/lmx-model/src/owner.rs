//! State of the guest owner daemon, as `lmx status` reports it.

use serde::{Deserialize, Serialize};

/// The guest owner daemon `lmxd` and what it is doing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    /// Release version of `lmxd`.
    pub version: String,
    /// Conditions derived from facts when `lmxd` was asked; none are stored.
    pub conditions: Vec<Condition>,
    /// Operations `lmxd` has accepted and not finished.
    pub operations: Vec<Operation>,
}

/// A condition of the guest that needs attention.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    /// Stable name, such as `DiskLow`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Explanation for people.
    pub message: String,
}

/// Name of the condition set while less than the platform minimum of the store disk is free.
pub const DISK_LOW: &str = "DiskLow";

/// Name of the condition set while the mounted generation is not built: an apply is needed.
pub const OUT_OF_DATE: &str = "OutOfDate";

/// Name of the condition set while the built generation is not booted: a restart is needed.
pub const RESTART_REQUIRED: &str = "RestartRequired";

/// Name of the condition set when the mounted generation is built, booted, healthy and finalized: the
/// only generation of the system profile, with the boot entries rewritten.
pub const CONVERGED: &str = "Converged";

/// Name of the condition set while the booted generation fails its health check.
pub const DEGRADED: &str = "Degraded";

/// Name of the condition set after the finalize of a healthy booted generation failed, until a later
/// try succeeds: removing the older generations or rewriting the boot entries. The generation works;
/// `lmxd` tries again later.
pub const FINALIZE_FAILED: &str = "FinalizeFailed";

/// One operation of `lmxd`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Operation {
    /// Name of the task that runs the operation, such as `store-collect-1`.
    pub task: String,
    /// Kind of the operation, such as `StoreCollect`.
    pub kind: String,
    /// Lifecycle phase, such as `pending` or `running`.
    pub phase: String,
    /// When the operation was requested, in Unix milliseconds.
    pub created_at: u64,
}
