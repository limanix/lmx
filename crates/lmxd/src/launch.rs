//! Starting `lmxd` tasks and waiting for them.

use std::time::Duration;

use solti::{
    core::SupervisorApi,
    model::{
        AdmissionPolicy, RestartPolicy, TaskId, TaskManifest, TaskPhase, TaskSpec, TaskStatus,
        TaskWorkload,
    },
};

/// How often a waiting caller checks its task.
const POLL: Duration = Duration::from_millis(100);

/// Where and how long a task runs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placement {
    /// Slot of the task.
    pub(crate) slot: &'static str,
    /// What happens when the slot is busy.
    pub(crate) admission: AdmissionPolicy,
    /// Longest an attempt may run; [`Duration::MAX`] for no limit.
    pub(crate) timeout: Duration,
}

/// How a task ended without success.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ended {
    /// Phase it ended in, such as `exhausted` or `canceled`.
    pub(crate) phase: TaskPhase,
    /// Exit status of its process, when the process exited.
    pub(crate) exit_code: Option<i32>,
    /// Why, for people.
    pub(crate) message: String,
}

/// Creates the task `name` that runs `workload` once.
pub(crate) async fn start(
    supervisor: &SupervisorApi,
    name: &str,
    workload: TaskWorkload,
    placement: Placement,
) -> Result<TaskId, String> {
    let timeout = u64::try_from(placement.timeout.as_millis()).unwrap_or(u64::MAX);
    let spec = TaskSpec::builder(placement.slot, workload, timeout)
        .restart(RestartPolicy::Never)
        .admission(placement.admission)
        .build()
        .map_err(|error| error.to_string())?;
    let manifest = TaskManifest::new(name, spec).map_err(|error| error.to_string())?;
    let task = supervisor
        .create_task(manifest)
        .await
        .map_err(|error| error.to_string())?;
    Ok(task.name().clone())
}

/// Whether a task with `status` will still run: pending or running, and built by its runner.
///
/// A task whose runner could not build it stays pending for good, so it does not run.
pub(crate) fn runs(status: &TaskStatus) -> bool {
    status.phase().is_active() && !status.reconciliation_failed()
}

/// Waits until the task `name` ends; an outcome other than success is [`Ended`].
pub(crate) async fn finished(supervisor: &SupervisorApi, name: &TaskId) -> Result<(), Ended> {
    let mut poll = tokio::time::interval(POLL);
    loop {
        poll.tick().await;
        let Some(task) = supervisor.get_task(name) else {
            return Err(Ended {
                phase: TaskPhase::Canceled,
                exit_code: None,
                message: format!("task {name} was removed"),
            });
        };
        let status = task.status();
        if status.reconciliation_failed() {
            return Err(Ended {
                phase: status.phase(),
                exit_code: None,
                message: status.reconciled().message().to_owned(),
            });
        }
        let phase = status.phase();
        if phase == TaskPhase::Succeeded {
            return Ok(());
        }
        if phase.is_terminal() {
            return Err(Ended {
                phase,
                exit_code: status.exit_code(),
                message: status
                    .error()
                    .map_or_else(|| format!("task {name} ended {phase}"), ToOwned::to_owned),
            });
        }
    }
}
