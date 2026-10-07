//! Task output in the journal.

use solti::{
    core::{TaskOutputEvent, TaskOutputSink},
    model::{OutputEvent, StreamKind},
};

/// Logs every output line of every task, so `journalctl -u lmx` shows what the tasks printed, as
/// `journalctl -u limanix-store-guard` showed the platform guard's output.
#[derive(Debug)]
pub(crate) struct Journal;

impl TaskOutputSink for Journal {
    fn on_event(&self, event: &TaskOutputEvent) {
        let task = event.task();
        match event.event() {
            OutputEvent::Chunk(chunk) => {
                let line = String::from_utf8_lossy(&chunk.line);
                match chunk.stream {
                    StreamKind::Stdout => {
                        tracing::info!(target: "lmxd::task", lmx_task = %task, "{line}")
                    }
                    StreamKind::Stderr => {
                        tracing::warn!(target: "lmxd::task", lmx_task = %task, "{line}")
                    }
                }
            }
            OutputEvent::RunFinished { exit_code, .. } => {
                tracing::info!(target: "lmxd::task", lmx_task = %task, ?exit_code, "task run finished");
            }
            _ => {}
        }
    }
}
