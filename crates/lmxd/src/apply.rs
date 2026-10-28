//! The apply operation: building a requested generation for the next boot.
//!
//! The host provides inputs at `/mnt/limanix/<generation>` and asks for them by generation. An apply
//! makes room in the store, runs `nixos-rebuild boot` as a `SystemApply` task, then installs the
//! generation's environment. The apply belongs to `lmxd`: a caller that disconnects only stops
//! following it, and asking again attaches to the running one. Its state lives in memory; after a
//! reboot the generation markers tell what was built.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as SyncMutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use lmx_model::{
    Apply, ApplyEvent, ApplyPhase, ApplyState, BuildFailure, CancelApply, ErrorBody, ErrorCode,
    Generations, OutputStream,
};
use serde_json::{Map, Value};
use solti::{
    core::SupervisorApi,
    model::{AdmissionPolicy, TaskId, TaskPhase},
};
use tokio::sync::{Mutex, broadcast, watch};

use crate::{
    capture::{Capture, Line},
    environment,
    launch::{self, Ended, Placement},
    paths::Paths,
    store::Store,
    tasks::{self, Kind},
};

/// Slot of the system tasks.
pub(crate) const SYSTEM_SLOT: &str = "system";

/// Messages kept for a follower that falls behind; older ones are reported as lagged.
const BACKLOG: usize = 1024;

/// What `nix` prints, in lowercase, when the disk is full.
const DISK_FULL: &str = "no space left on device";

/// A message to the followers of an apply.
#[derive(Clone, Debug)]
pub(crate) enum Message {
    /// Progress.
    Event(ApplyEvent),
    /// How the apply ended; nothing follows.
    Outcome(Result<Apply, ErrorBody>),
}

/// How a caller joins an apply.
#[derive(Debug)]
pub(crate) enum Joined {
    /// The apply runs; its messages arrive until the outcome.
    Following(broadcast::Receiver<Message>),
    /// The apply has an outcome already.
    Done(Result<Apply, ErrorBody>),
}

/// Followers' channel and outcome of a run, under one lock so a new follower misses neither.
#[derive(Debug)]
struct Progress {
    /// Channel to the followers.
    messages: broadcast::Sender<Message>,
    /// The outcome, once there is one.
    outcome: Option<Result<Apply, ErrorBody>>,
}

/// One apply of a generation.
#[derive(Debug)]
pub(crate) struct Run {
    /// The generation being applied.
    generation: String,
    /// Immutable input directory of the accepted generation.
    inputs: PathBuf,
    /// Name of the run and of its build task.
    name: String,
    /// Followers and outcome.
    progress: SyncMutex<Progress>,
    /// Becomes `true` when the apply is cancelled.
    cancelled: watch::Sender<bool>,
    /// The `SystemApply` task, once it exists.
    task: SyncMutex<Option<TaskId>>,
}

impl Run {
    /// A run of `generation` named `name`, without followers.
    fn new(generation: &str, name: String, inputs: PathBuf) -> Self {
        Self {
            generation: generation.to_owned(),
            inputs,
            name,
            progress: SyncMutex::new(Progress {
                messages: broadcast::channel(BACKLOG).0,
                outcome: None,
            }),
            cancelled: watch::Sender::new(false),
            task: SyncMutex::new(None),
        }
    }

    /// The followers and outcome, locked.
    fn progress(&self) -> std::sync::MutexGuard<'_, Progress> {
        self.progress.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tells the followers about `event`.
    fn event(&self, event: ApplyEvent) {
        let _ = self.progress().messages.send(Message::Event(event));
    }

    /// Records the outcome and tells the followers.
    fn finish(&self, outcome: Result<Apply, ErrorBody>) {
        let mut progress = self.progress();
        progress.outcome = Some(outcome.clone());
        let _ = progress.messages.send(Message::Outcome(outcome));
    }

    /// Joins the run as a follower, or returns its outcome.
    fn join(&self) -> Joined {
        let progress = self.progress();
        match &progress.outcome {
            Some(outcome) => Joined::Done(outcome.clone()),
            None => Joined::Following(progress.messages.subscribe()),
        }
    }

    /// Whether the run has no outcome yet.
    fn running(&self) -> bool {
        self.progress().outcome.is_none()
    }

    /// Whether the run was cancelled.
    fn is_cancelled(&self) -> bool {
        *self.cancelled.borrow()
    }

    /// The `SystemApply` task, once it exists.
    fn task(&self) -> Option<TaskId> {
        self.task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// What an apply request does.
#[derive(Debug)]
enum Decision<'a> {
    /// An apply of the requested generation runs: attach to it.
    Join(&'a Arc<Run>),
    /// An apply of another generation runs: stop it, then decide again.
    Replace(&'a Arc<Run>),
    /// The requested generation is built: install its environment before answering.
    Built,
    /// Start an apply.
    Start,
}

/// What a request for `requested` does, given the generation markers and the running apply.
fn decide<'a>(
    requested: &str,
    generations: &Generations,
    running: Option<&'a Arc<Run>>,
) -> Decision<'a> {
    match running {
        Some(run) if run.generation == requested => Decision::Join(run),
        Some(run) => Decision::Replace(run),
        None if generations.built.as_deref() == Some(requested) => Decision::Built,
        None => Decision::Start,
    }
}

/// Applies generations, one at a time.
pub(crate) struct Applier {
    /// Supervisor of the build task.
    supervisor: Arc<SupervisorApi>,
    /// Store operations, for the reserve before a build.
    store: Arc<Store>,
    /// Copies of the build's output.
    capture: Arc<Capture>,
    /// Guest locations.
    paths: Paths,
    /// Group of the installed environment files.
    gid: u32,
    /// Number of the last run, for unique task names.
    created: AtomicU64,
    /// The latest apply.
    current: Mutex<Option<Arc<Run>>>,
    /// Set when `lmxd` stops; an apply that ends from then on says so instead of how its task ended.
    stopping: AtomicBool,
}

impl Applier {
    /// An applier that runs its builds in `supervisor`.
    pub(crate) fn new(
        supervisor: Arc<SupervisorApi>,
        store: Arc<Store>,
        capture: Arc<Capture>,
        paths: Paths,
        gid: u32,
    ) -> Self {
        Self {
            supervisor,
            store,
            capture,
            paths,
            gid,
            created: AtomicU64::new(0),
            current: Mutex::new(None),
            stopping: AtomicBool::new(false),
        }
    }

    /// Starts the apply of `generation`, joins the running one, or answers at once.
    ///
    /// The candidate directory must have the requested generation's marker. A built generation
    /// retries environment installation without another build. An apply of another generation is
    /// cancelled first, and the decision made again once it stopped.
    pub(crate) async fn apply(self: &Arc<Self>, generation: &str) -> Result<Joined, ErrorBody> {
        let mut current = self.current.lock().await;
        loop {
            let (inputs, generations) = self
                .paths
                .candidate(generation)
                .map_err(|found| mismatch(generation, found.as_deref()))?;
            let running = current.clone().filter(|run| run.running());
            match decide(generation, &generations, running.as_ref()) {
                Decision::Join(run) => return Ok(run.join()),
                Decision::Replace(run) => self.stop(run).await,
                Decision::Built => {
                    self.install_environment(&inputs)?;
                    return Ok(Joined::Done(Ok(restart_required(generation))));
                }
                Decision::Start => {
                    let number = self.created.fetch_add(1, Ordering::Relaxed) + 1;
                    let name = format!("{}-{number}", Kind::SystemApply.task_prefix());
                    let run = Arc::new(Run::new(generation, name, inputs));
                    let joined = run.join();
                    *current = Some(Arc::clone(&run));
                    tokio::spawn(Arc::clone(self).drive(run));
                    return Ok(joined);
                }
            }
        }
    }

    /// Cancels the running apply of `generation` and waits until it stops.
    pub(crate) async fn cancel(&self, generation: &str) -> CancelApply {
        let current = self.current.lock().await.clone();
        let Some(run) = current.filter(|run| run.generation == generation && run.running()) else {
            return CancelApply { cancelled: false };
        };
        self.stop(&run).await;
        CancelApply { cancelled: true }
    }

    /// Marks `lmxd` as stopping, before the supervisor cancels the tasks.
    pub(crate) fn stopping(&self) {
        self.stopping.store(true, Ordering::Relaxed);
    }

    /// Cancels `run`, and its build task once it exists, and waits until the run has an outcome.
    async fn stop(&self, run: &Run) {
        let joined = run.join();
        run.cancelled.send_replace(true);
        if let Some(task) = run.task() {
            let _ = self.supervisor.cancel_task(&task).await;
        }
        if let Joined::Following(mut messages) = joined {
            loop {
                match messages.recv().await {
                    Ok(Message::Outcome(_)) | Err(broadcast::error::RecvError::Closed) => break,
                    Ok(Message::Event(_)) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                }
            }
        }
    }

    /// Runs the steps of `run` and records its outcome.
    async fn drive(self: Arc<Self>, run: Arc<Run>) {
        let (task, kind, generation) = (
            run.name.as_str(),
            Kind::SystemApply.name(),
            run.generation.as_str(),
        );
        tracing::info!(
            lmx_task = task,
            lmx_kind = kind,
            lmx_generation = generation,
            "applying generation {generation}"
        );
        let steps = tokio::spawn({
            let applier = Arc::clone(&self);
            let run = Arc::clone(&run);
            async move { applier.steps(&run).await }
        });
        let mut outcome = steps.await.unwrap_or_else(|error| {
            Err(failure(
                ErrorCode::ApplyBuildFailed,
                format!("The apply stopped unexpectedly: {error}"),
            ))
        });

        if outcome.is_err() && self.stopping.load(Ordering::Relaxed) {
            outcome = Err(failure(
                ErrorCode::OwnerUnavailable,
                "lmxd stopped before the apply ended; apply the generation again.".into(),
            ));
        }
        match &outcome {
            Ok(_) => tracing::info!(
                lmx_task = task,
                lmx_kind = kind,
                lmx_generation = generation,
                "built generation {generation}"
            ),
            Err(error) if error.code == ErrorCode::ApplyCancelled => tracing::info!(
                lmx_task = task,
                lmx_kind = kind,
                lmx_generation = generation,
                "apply of generation {generation} cancelled"
            ),
            Err(error) => tracing::warn!(
                lmx_task = task,
                lmx_kind = kind,
                lmx_generation = generation,
                error = error.message,
                "apply of generation {generation} failed: {}",
                error.message
            ),
        }
        run.finish(outcome);
    }

    /// Reserve and build, then finish installing the environment after a successful build.
    async fn steps(&self, run: &Run) -> Result<Apply, ErrorBody> {
        stop_if_cancelled(run)?;
        environment::validate(&run.inputs).map_err(environment_failed)?;

        run.event(ApplyEvent::Phase {
            phase: ApplyPhase::Reserve,
        });

        let mut cancel = run.cancelled.subscribe();
        tokio::select! {
            reserved = self.store.reserve() => {
                if let Err(warning) = reserved {
                    run.event(ApplyEvent::Warning {
                        code: warning.code,
                        message: warning.message,
                    });
                }
            }
            _ = cancel.wait_for(|cancelled| *cancelled) => return Err(cancelled()),
        }

        run.event(ApplyEvent::Phase {
            phase: ApplyPhase::Build,
        });
        let built = self.build(run).await?;
        run.event(ApplyEvent::Phase {
            phase: ApplyPhase::Environment,
        });
        self.install_environment(&run.inputs)?;
        Ok(built)
    }

    /// Installs the environment after a successful build, including a retry of a built generation.
    fn install_environment(&self, inputs: &Path) -> Result<(), ErrorBody> {
        environment::install(&self.paths, inputs, self.gid).map_err(environment_failed)
    }

    /// Runs `nixos-rebuild boot` and passes its output to the followers.
    async fn build(&self, run: &Run) -> Result<Apply, ErrorBody> {
        let name = run.name.as_str();
        let mut lines = self.capture.listen(name);
        let placement = Placement {
            slot: SYSTEM_SLOT,
            admission: AdmissionPolicy::Queue,
            timeout: Duration::MAX,
        };
        let started = match tasks::apply(&run.generation) {
            Ok(workload) => launch::start(&self.supervisor, name, workload, placement).await,
            Err(error) => Err(error.to_string()),
        };
        let task = started.map_err(|error| {
            self.capture.forget(name);
            failure(
                ErrorCode::ApplyBuildFailed,
                format!("The build cannot start: {error}"),
            )
        })?;
        *run.task.lock().unwrap_or_else(PoisonError::into_inner) = Some(task.clone());
        tracing::info!(
            lmx_task = name,
            lmx_kind = Kind::SystemApply.name(),
            lmx_generation = run.generation,
            "building generation {}",
            run.generation
        );
        if run.is_cancelled() {
            let _ = self.supervisor.cancel_task(&task).await;
        }

        let mut full = false;
        let finished = launch::finished(&self.supervisor, &task);
        tokio::pin!(finished);
        let result = loop {
            tokio::select! {
                result = &mut finished => break result,
                Some(line) = lines.recv() => full |= forward(run, line),
            }
        };
        self.capture.forget(name);
        while let Ok(line) = lines.try_recv() {
            full |= forward(run, line);
        }

        match result {
            Ok(()) => Ok(restart_required(&run.generation)),
            Err(ended) if run.is_cancelled() || ended.phase == TaskPhase::Canceled => {
                Err(cancelled())
            }
            Err(ended) => Err(build_failed(&ended, self.store.shortage(full))),
        }
    }
}

/// Passes a line of the build to the followers; returns whether it says the disk is full.
fn forward(run: &Run, line: Line) -> bool {
    let full = line.text.to_lowercase().contains(DISK_FULL);
    run.event(ApplyEvent::Output {
        stream: if line.stderr {
            OutputStream::Stderr
        } else {
            OutputStream::Stdout
        },
        line: line.text,
        truncated: line.truncated,
    });
    full
}

/// The answer for a generation that is built and needs a restart.
fn restart_required(generation: &str) -> Apply {
    Apply {
        generation: generation.to_owned(),
        state: ApplyState::RestartRequired,
    }
}

/// A failure with `code` and `message`, without details.
fn failure(code: ErrorCode, message: String) -> ErrorBody {
    ErrorBody {
        code,
        message,
        details: Map::new(),
    }
}

/// An environment file could not be read, staged or installed.
fn environment_failed(error: std::io::Error) -> ErrorBody {
    failure(
        ErrorCode::ApplyEnvironmentFailed,
        format!("The environment files cannot be installed: {error}"),
    )
}

/// The failure of an apply whose candidate marker does not match the requested generation.
fn mismatch(requested: &str, mounted: Option<&str>) -> ErrorBody {
    let mut error = failure(
        ErrorCode::GenerationMismatch,
        format!(
            "Generation {requested} does not match the marker at /mnt/limanix/{requested} ({}).",
            mounted.unwrap_or("none")
        ),
    );
    error.details.insert("requested".into(), requested.into());
    error
        .details
        .insert("mounted".into(), mounted.map_or(Value::Null, Value::from));
    error
}

/// The failure of a cancelled apply.
fn cancelled() -> ErrorBody {
    failure(ErrorCode::ApplyCancelled, "The apply was cancelled.".into())
}

/// Stops the apply when it was cancelled.
fn stop_if_cancelled(run: &Run) -> Result<(), ErrorBody> {
    if run.is_cancelled() {
        Err(cancelled())
    } else {
        Ok(())
    }
}

/// The failure of a build that ended as `ended`; `disk` is the usage of a full disk.
fn build_failed(ended: &Ended, disk: Option<lmx_model::DiskUsage>) -> ErrorBody {
    let message = ended.exit_code.map_or_else(
        || format!("nixos-rebuild failed: {}", ended.message),
        |code| format!("nixos-rebuild failed with exit status {code}."),
    );
    let details = BuildFailure {
        exit_code: ended.exit_code,
        disk,
    };
    let mut error = failure(ErrorCode::ApplyBuildFailed, message);
    if let Ok(Value::Object(details)) = serde_json::to_value(details) {
        error.details = details;
    }
    error
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn decides_between_join_replace_built_and_start() {
        let generations = |desired: &str, built: &str| Generations {
            desired: Some(desired.to_owned()),
            built: Some(built.to_owned()),
            booted: Some("g0".to_owned()),
        };
        let g1 = Arc::new(Run::new(
            "g1",
            "system-apply-1".into(),
            "/mnt/limanix/g1".into(),
        ));
        let g2 = Arc::new(Run::new(
            "g2",
            "system-apply-2".into(),
            "/mnt/limanix/g2".into(),
        ));
        let mounted_g2 = generations("g2", "g1");
        assert!(matches!(decide("g3", &mounted_g2, None), Decision::Start));
        assert!(matches!(
            decide("g2", &mounted_g2, Some(&g2)),
            Decision::Join(_)
        ));
        assert!(matches!(
            decide("g2", &mounted_g2, Some(&g1)),
            Decision::Replace(_)
        ));
        assert!(matches!(decide("g2", &mounted_g2, None), Decision::Start));
        assert!(matches!(
            decide("g2", &generations("g2", "g2"), None),
            Decision::Built
        ));
        assert!(
            matches!(
                decide("g1", &generations("g1", "g1"), Some(&g2)),
                Decision::Replace(_)
            ),
            "a build of another generation never outlives the answer"
        );
    }

    #[test]
    fn a_mismatch_names_both_generations() {
        let error = mismatch("g2", Some("g1"));
        assert_eq!(error.code, ErrorCode::GenerationMismatch);
        assert_eq!(
            Value::Object(error.details),
            json!({"requested": "g2", "mounted": "g1"})
        );
        assert_eq!(
            Value::Object(mismatch("g2", None).details)["mounted"],
            Value::Null
        );
    }

    #[test]
    fn a_failed_build_reports_its_exit_status() {
        let ended = Ended {
            phase: TaskPhase::Exhausted,
            exit_code: Some(1),
            message: "process exited with non-zero code: 1".into(),
        };
        let error = build_failed(&ended, None);
        assert_eq!(error.message, "nixos-rebuild failed with exit status 1.");
        assert_eq!(Value::Object(error.details), json!({"exit_code": 1}));
    }

    #[tokio::test]
    async fn a_follower_that_joins_first_sees_every_message() {
        let run = Run::new("g1", "system-apply-1".into(), "/mnt/limanix/g1".into());
        let Joined::Following(mut messages) = run.join() else {
            panic!("a new run has no outcome");
        };
        run.event(ApplyEvent::Phase {
            phase: ApplyPhase::Environment,
        });
        run.finish(Ok(restart_required("g1")));

        assert!(matches!(
            messages.recv().await,
            Ok(Message::Event(ApplyEvent::Phase { .. }))
        ));
        assert!(matches!(messages.recv().await, Ok(Message::Outcome(Ok(_)))));
        assert!(matches!(run.join(), Joined::Done(Ok(_))));
    }
}
