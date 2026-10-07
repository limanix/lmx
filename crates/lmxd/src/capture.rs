//! Copies of task output for `lmxd` itself.
//!
//! Solti's output stream is live: it begins when a subscriber arrives and misses what came before.
//! `lmxd` needs every line of some of its tasks, such as the build its apply followers read, or the
//! reason a health check failed. [`Tee`] wraps the publisher the runner writes to and copies the
//! lines of the tasks someone listens to; registering before the task exists loses nothing.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use solti::{
    model::{StreamKind, TaskId},
    runner::{
        OutputChunkRef, OutputPublisher, OutputPublisherHandle, OutputSink, request_output_sink,
    },
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// One line a task printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    /// Whether it came from standard error.
    pub(crate) stderr: bool,
    /// The line; invalid UTF-8 is replaced.
    pub(crate) text: String,
    /// Whether the end of a long line was cut.
    pub(crate) truncated: bool,
}

/// Listeners of task output, by task name.
#[derive(Debug, Default)]
pub(crate) struct Capture {
    /// Where the lines of each listened-to task go.
    listeners: Mutex<HashMap<String, UnboundedSender<Line>>>,
}

impl Capture {
    /// Receives the output of the task `name` from now on; call it before the task is created.
    pub(crate) fn listen(&self, name: &str) -> UnboundedReceiver<Line> {
        let (sender, receiver) = unbounded_channel();
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), sender);
        receiver
    }

    /// Stops copying the output of the task `name`.
    pub(crate) fn forget(&self, name: &str) {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
    }

    /// Where the lines of the task `name` go, if someone listens.
    fn listener(&self, name: &TaskId) -> Option<UnboundedSender<Line>> {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name.as_str())
            .cloned()
    }
}

/// Output publisher that passes every line on and copies the lines of listened-to tasks.
pub(crate) struct Tee {
    /// The publisher the supervisor gave the runner.
    pub(crate) inner: OutputPublisherHandle,
    /// Listeners of task output.
    pub(crate) capture: Arc<Capture>,
}

impl OutputPublisher for Tee {
    fn sink_for(&self, task_name: &TaskId, generation: u64, attempt: u32) -> Option<OutputSink> {
        let inner = request_output_sink(&self.inner, task_name, generation, attempt);
        let Some(listener) = self.capture.listener(task_name) else {
            return inner;
        };
        Some(OutputSink::new_borrowed(
            generation,
            attempt,
            move |chunk: OutputChunkRef<'_>| {
                let stderr = chunk.stream() == StreamKind::Stderr;
                if let Some(inner) = &inner {
                    match (stderr, chunk.truncated()) {
                        (false, false) => inner.stdout_line_bytes(chunk.line()),
                        (false, true) => inner.stdout_line_bytes_truncated(chunk.line()),
                        (true, false) => inner.stderr_line_bytes(chunk.line()),
                        (true, true) => inner.stderr_line_bytes_truncated(chunk.line()),
                    }
                }
                let _ = listener.send(Line {
                    stderr,
                    text: String::from_utf8_lossy(chunk.line()).into_owned(),
                    truncated: chunk.truncated(),
                });
            },
        ))
    }
}
