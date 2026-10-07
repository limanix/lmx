//! Answers and events of `lmx apply`.

use serde::{Deserialize, Serialize};

use crate::{DiskUsage, ErrorCode};

/// Phase of an apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyPhase {
    /// Installing the environment files of the generation.
    Environment,
    /// Making room in the store.
    Reserve,
    /// Building the generation for the next boot.
    Build,
}

/// Stream of a line the build printed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// One event of `lmx apply --follow --json`; the contract envelope with the outcome follows the last
/// one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ApplyEvent {
    /// A phase started.
    Phase {
        /// The phase.
        phase: ApplyPhase,
    },
    /// A line the build printed.
    Output {
        /// Where the line was printed.
        stream: OutputStream,
        /// The line, without its newline; invalid UTF-8 is replaced.
        line: String,
        /// Whether the end of a long line was cut.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        truncated: bool,
    },
    /// A problem that does not stop the apply, such as a low disk.
    Warning {
        /// Code of the problem.
        code: ErrorCode,
        /// Explanation for people.
        message: String,
    },
    /// The follower fell behind and missed events.
    Lagged {
        /// Number of missed events.
        skipped: u64,
    },
}

/// State of an apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyState {
    /// The apply runs; follow it, or ask again later.
    Running,
    /// The generation is built for the next boot; restart the VM to boot it.
    RestartRequired,
}

/// Answer of `lmx apply`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Apply {
    /// The generation applied.
    pub generation: String,
    /// Where it stands.
    pub state: ApplyState,
}

/// Answer of `lmx apply cancel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelApply {
    /// Whether an apply of the generation was running and is now cancelled.
    pub cancelled: bool,
}

/// Details of an `apply.build_failed` failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildFailure {
    /// Exit status of `nixos-rebuild`, when it exited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Usage of the store disk, when the failure looks like a full disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk: Option<DiskUsage>,
}

#[cfg(test)]
mod tests {
    use crate::{Apply, ApplyEvent, BuildFailure, CONTRACT_VERSION, Envelope, ErrorCode};

    /// The published apply examples decode and encode without loss.
    #[test]
    fn contract_examples_round_trip() {
        let example = include_str!("../../../contract/v1/apply-restart-required.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<Apply> = serde_json::from_str(example).expect("example decodes");
        assert_eq!(envelope.contract, CONTRACT_VERSION);
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);

        let example = include_str!("../../../contract/v1/apply-build-failed.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<Apply> = serde_json::from_str(example).expect("example decodes");
        let error = envelope.error.clone().expect("a failure");
        assert_eq!(error.code, ErrorCode::ApplyBuildFailed);
        let failure: BuildFailure =
            serde_json::from_value(error.details.into()).expect("details are a build failure");
        assert_eq!(failure.exit_code, Some(1));
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);
    }

    /// Every line of the follow example but the last is an event; the last is the envelope.
    #[test]
    fn follow_example_is_events_then_an_envelope() {
        let example = include_str!("../../../contract/v1/apply-follow.jsonl");
        let lines: Vec<&str> = example.lines().collect();
        let (last, events) = lines.split_last().expect("lines");
        for line in events {
            let original: serde_json::Value = serde_json::from_str(line).expect("JSON");
            let event: ApplyEvent = serde_json::from_str(line).expect("an event");
            assert_eq!(serde_json::to_value(&event).expect("encode"), original);
        }
        let envelope: Envelope<Apply> = serde_json::from_str(last).expect("an envelope");
        assert!(envelope.ok);
    }
}
