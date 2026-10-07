//! Failure of one fact reader.

use std::{io, process::ExitStatus, time::Duration};

/// Failure of one fact reader.
///
/// Callers report the failure next to the other facts instead of stopping, so every variant renders
/// a complete sentence for people.
#[derive(Debug, thiserror::Error)]
pub enum FactError {
    /// A system call on a file or file system failed.
    #[error("cannot read {what}: {source}")]
    Io {
        /// What was read.
        what: &'static str,
        /// Underlying I/O failure.
        #[source]
        source: io::Error,
    },
    /// A program could not be started.
    #[error("cannot run {program}: {source}")]
    Spawn {
        /// Program path as configured.
        program: String,
        /// Underlying I/O failure.
        #[source]
        source: io::Error,
    },
    /// A program ran and reported failure.
    #[error("{program} failed ({status}){}", stderr_suffix(stderr))]
    Command {
        /// Program path as configured.
        program: String,
        /// Exit status.
        status: ExitStatus,
        /// Last part of the program's standard error.
        stderr: String,
    },
    /// A program did not finish in time.
    #[error("{program} did not finish within {timeout:?}")]
    Timeout {
        /// Program path as configured.
        program: String,
        /// How long the reader waited.
        timeout: Duration,
    },
    /// A program's output or a system file did not have the expected shape.
    #[error("unexpected {what} output: {detail}")]
    Parse {
        /// Which output was parsed.
        what: &'static str,
        /// Parser explanation.
        detail: String,
    },
}

/// Appends standard error to a message when there is any.
fn stderr_suffix(stderr: &str) -> String {
    if stderr.is_empty() {
        String::new()
    } else {
        format!(": {stderr}")
    }
}
