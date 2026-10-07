//! Records of `lmxd` tasks in the system journal, read with `journalctl -o json`.
//!
//! `lmxd` writes the output of its tasks with the fields `LMX_TASK` and `LMX_KIND`, and apply events
//! also with `LMX_GENERATION`. The journal is readable by root and the groups `wheel`, `adm` and
//! `systemd-journal`; other users see none of these records.

use std::{path::Path, time::Duration};

use serde_json::Value;

use crate::{FactError, command};

/// Longest wait for `journalctl`; a long build writes many records.
const TIMEOUT: Duration = Duration::from_secs(10);

/// One record of an `lmxd` task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Task that wrote it, such as `system-apply-3`; empty for an event outside a task.
    pub task: String,
    /// Process that wrote it; a restarted `lmxd` numbers its tasks from 1 again.
    pub pid: u32,
    /// Generation of an apply, when the record names one.
    pub generation: Option<String>,
    /// The line or event.
    pub message: String,
    /// When it was written, in microseconds since the Unix epoch.
    pub time: u64,
}

/// Which boot to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boot {
    /// The running boot.
    Current,
    /// The boot before it, such as the one that built the running generation.
    Previous,
}

/// Records of the task kind `kind` in `boot`, in the order they were written, and whether the
/// journal of the system was readable to the caller.
///
/// Only records of root count, since any user may write a record with `LMX_KIND`. A boot that the
/// journal does not have has no records.
pub fn records(
    journalctl: &Path,
    kind: &str,
    boot: Boot,
) -> Result<(Vec<Record>, bool), FactError> {
    let boot = match boot {
        Boot::Current => "-b0",
        Boot::Previous => "-b-1",
    };
    let filter = format!("LMX_KIND={kind}");
    let answer = command::output_and_errors(
        journalctl,
        &[
            "-o",
            "json",
            "--no-pager",
            "--all",
            "--output-fields=MESSAGE,LMX_TASK,LMX_GENERATION,_PID",
            boot,
            "_UID=0",
            &filter,
        ],
        TIMEOUT,
    );
    let (output, errors) = match answer {
        // journalctl fails when it can open no journal, or when the boot is not in it.
        Err(FactError::Command { stderr, .. }) if unreadable(&stderr) => {
            return Ok((Vec::new(), false));
        }
        Err(FactError::Command { stderr, .. }) if missing_boot(&stderr) => {
            return Ok((Vec::new(), true));
        }
        answer => answer?,
    };
    Ok((
        parse(&String::from_utf8_lossy(&output)),
        !unreadable(&errors),
    ))
}

/// Whether journalctl says on standard error that the caller cannot see the system's messages.
fn unreadable(errors: &str) -> bool {
    errors.contains("not seeing messages") || errors.contains("insufficient permissions")
}

/// Whether journalctl says that the journal does not have the requested boot.
fn missing_boot(errors: &str) -> bool {
    errors.contains("No journal boot entry") || errors.contains("No such boot ID")
}

/// Parses `journalctl -o json` output, one object per line; lines that are not objects are skipped.
pub fn parse(output: &str) -> Vec<Record> {
    output
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .map(|record| Record {
            task: text(&record["LMX_TASK"]).unwrap_or_default(),
            pid: text(&record["_PID"])
                .and_then(|pid| pid.parse().ok())
                .unwrap_or(0),
            generation: text(&record["LMX_GENERATION"]).filter(|generation| !generation.is_empty()),
            message: text(&record["MESSAGE"]).unwrap_or_default(),
            time: text(&record["__REALTIME_TIMESTAMP"])
                .and_then(|time| time.parse().ok())
                .unwrap_or(0),
        })
        .collect()
}

/// A field value: journalctl writes text as a string and other data as an array of bytes.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(bytes) => {
            let bytes: Option<Vec<u8>> = bytes
                .iter()
                .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
                .collect();
            Some(String::from_utf8_lossy(&bytes?).into_owned())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_task_records_with_text_and_bytes() {
        let output = concat!(
            r#"{"MESSAGE":"building the system configuration...","_PID":"812","LMX_TASK":"system-apply-3","__REALTIME_TIMESTAMP":"1791374400000000"}"#,
            "\n",
            r#"{"MESSAGE":[104,105,255],"_PID":"812","LMX_TASK":"system-apply-3","__REALTIME_TIMESTAMP":"1791374400000001"}"#,
            "\n",
            r#"{"MESSAGE":"built the generation","_PID":"812","LMX_TASK":"system-apply-3","LMX_GENERATION":"g2","__REALTIME_TIMESTAMP":"1791374400000002"}"#,
            "\n-- No entries --\n",
        );
        let records = parse(output);
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].pid, 812);
        assert_eq!(records[1].message, "hi\u{fffd}");
        assert_eq!(records[2].generation.as_deref(), Some("g2"));
        assert_eq!(records[2].time, 1_791_374_400_000_002);
    }

    #[test]
    fn tells_an_unreadable_journal_from_a_missing_boot() {
        assert!(unreadable(
            "No journal files were opened due to insufficient permissions."
        ));
        assert!(unreadable(
            "Hint: You are currently not seeing messages from other users and the system."
        ));
        assert!(missing_boot(
            "No journal boot entry found for the specified boot (-1)."
        ));
        assert!(!unreadable(
            "Failed to open the journal: Input/output error"
        ));
    }
}
