//! Task output in the journal.
//!
//! Every line carries the fields `LMX_TASK` and `LMX_KIND`, such as `system-apply-3` and
//! `SystemApply`, by which `lmx logs` finds the runs of a kind, also after a reboot. `lmxd`
//! writes its journal fields without a prefix.

use solti::{
    core::{TaskOutputEvent, TaskOutputSink},
    model::{OutputEvent, StreamKind},
};

use crate::tasks::Kind;

/// Logs every output line of every task: `journalctl -u lmx` shows what the tasks printed, as
/// `journalctl -u limanix-store-guard` showed the platform guard's output.
#[derive(Debug)]
pub(crate) struct Journal;

impl TaskOutputSink for Journal {
    fn on_event(&self, event: &TaskOutputEvent) {
        let task = event.task();
        let kind = Kind::of_task(task.as_str()).map_or("", Kind::name);
        match event.event() {
            OutputEvent::Chunk(chunk) => {
                let line = String::from_utf8_lossy(&chunk.line);
                match chunk.stream {
                    StreamKind::Stdout => {
                        tracing::info!(target: "lmxd::task", lmx_task = %task, lmx_kind = kind, "{line}")
                    }
                    StreamKind::Stderr => {
                        tracing::warn!(target: "lmxd::task", lmx_task = %task, lmx_kind = kind, "{line}")
                    }
                }
            }
            OutputEvent::RunFinished {
                exit_code: Some(code),
                ..
            } => {
                tracing::info!(
                    target: "lmxd::task",
                    lmx_task = %task,
                    lmx_kind = kind,
                    exit_code = code,
                    "task run finished with exit status {code}"
                );
            }
            OutputEvent::RunFinished { .. } => {
                tracing::info!(target: "lmxd::task", lmx_task = %task, lmx_kind = kind, "task run finished");
            }
            _ => {}
        }
    }
}
