//! The generation observer: health and finalize of a booted generation.
//!
//! After the host restarts the VM into a new generation, the system `lmxd` checks that the
//! generation works, then finalizes it: the older generations of the system profile are removed and
//! the boot entries rewritten, as the host's prune did after its ready check. A generation that fails
//! its check is `Degraded` and keeps the older generations for a rollback; it is checked again every
//! minute. A finalize that fails is `FinalizeFailed`: the generation works, and the finalize is tried
//! again later. The check judges an update; once the generation is finalized and healthy, it stops.

use std::{
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use lmx_model::{
    CONVERGED, Condition, DEGRADED, FINALIZE_FAILED, Generations, OUT_OF_DATE, RESTART_REQUIRED,
};
use solti::{
    core::SupervisorApi,
    model::{AdmissionPolicy, ModelResult, TaskWorkload},
};

use crate::{
    apply::SYSTEM_SLOT,
    capture::Capture,
    launch::{self, Placement},
    paths::Paths,
    store::Store,
    tasks::{self, Kind, Priority},
};

/// Slot of the health check, apart from the system slot so an apply never waits for it.
const HEALTH_SLOT: &str = "health";

/// Longest a health check may run; a check that hangs, such as on a stuck mount, fails.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// Longest a finalize may run: well within the 10 minutes the host waits after a restart, so a stuck
/// finalize is reported as failed before that wait ends.
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// When the observer looks at the generations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObserverSchedule {
    /// Wait before the first look, so the system can settle after boot.
    pub first: Duration,
    /// Wait after each look before the next one.
    pub every: Duration,
    /// Wait after a failed finalize before trying again.
    pub retry: Duration,
}

impl ObserverSchedule {
    /// The system daemon's schedule: 30 seconds after start, then every minute; a failed finalize
    /// is retried after 15 minutes.
    #[must_use]
    pub fn system() -> Self {
        Self {
            first: Duration::from_secs(30),
            every: Duration::from_secs(60),
            retry: Duration::from_secs(15 * 60),
        }
    }
}

/// Result of the last health check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Health {
    /// Not checked for the booted generation.
    Unknown,
    /// The last check passed.
    Healthy,
    /// The last check failed, for the reason given.
    Unhealthy(String),
}

/// Where the finalize of the booted generation stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Finalize {
    /// None runs, and none failed.
    Idle,
    /// A finalize runs; the boot entries may still list removed generations.
    Running,
    /// The last finalize failed.
    Failed {
        /// When it may run again.
        retry: Instant,
        /// The last line it printed, or why it ended.
        reason: String,
    },
}

/// Watches the booted generation.
pub(crate) struct Observer {
    /// Supervisor of the check and finalize tasks.
    supervisor: Arc<SupervisorApi>,
    /// Store operations, for the collection after finalize.
    store: Arc<Store>,
    /// Copies of the tasks' output, for failure reasons.
    capture: Arc<Capture>,
    /// Guest locations.
    paths: Paths,
    /// Home of the development account, which must be mounted.
    home: String,
    /// Result of the last health check.
    health: Mutex<Health>,
    /// Where the finalize stands.
    finalize: Mutex<Finalize>,
    /// Number of the last task, for unique task names.
    created: AtomicU64,
}

impl Observer {
    /// An observer that runs its tasks in `supervisor`.
    pub(crate) fn new(
        supervisor: Arc<SupervisorApi>,
        store: Arc<Store>,
        capture: Arc<Capture>,
        paths: Paths,
        home: String,
    ) -> Self {
        Self {
            supervisor,
            store,
            capture,
            paths,
            home,
            health: Mutex::new(Health::Unknown),
            finalize: Mutex::new(Finalize::Idle),
            created: AtomicU64::new(0),
        }
    }

    /// Result of the last health check.
    pub(crate) fn health(&self) -> Health {
        self.health
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Records the result of a health check.
    fn set_health(&self, health: Health) {
        *self.health.lock().unwrap_or_else(PoisonError::into_inner) = health;
    }

    /// Where the finalize stands.
    pub(crate) fn finalize(&self) -> Finalize {
        self.finalize
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Records where the finalize stands.
    fn set_finalize(&self, finalize: Finalize) {
        *self.finalize.lock().unwrap_or_else(PoisonError::into_inner) = finalize;
    }

    /// One look: check a settled generation that is not finalized, and finalize it when healthy.
    ///
    /// A failed finalize is tried again after `retry`.
    pub(crate) async fn tick(&self, retry: Duration) {
        let (generations, _) = lmx_facts::generations::read(&self.paths.generations());
        let Some(generation) = settled(&generations).map(ToOwned::to_owned) else {
            self.set_health(Health::Unknown);
            self.set_finalize(Finalize::Idle);
            return;
        };
        let kept = match lmx_facts::generations::system_generations(&self.paths.profiles()) {
            Ok(kept) => kept,
            Err(error) => {
                tracing::warn!(%error, "the system profile generations cannot be read");
                return;
            }
        };
        let Step::Check { finalize } = step(kept, &self.health(), &self.finalize(), Instant::now())
        else {
            return;
        };

        let health = self.check(&generation).await;
        let healthy = health == Health::Healthy;
        self.set_health(health);
        if !(healthy && finalize) {
            return;
        }
        // The check takes a while: an apply may have built another generation meanwhile.
        let (generations, _) = lmx_facts::generations::read(&self.paths.generations());
        if settled(&generations) != Some(generation.as_str()) {
            return;
        }
        self.set_finalize(Finalize::Running);
        let task = self.name(Kind::SystemFinalize);
        let kind = Kind::SystemFinalize.name();
        match self
            .run(&task, Kind::SystemFinalize, tasks::finalize())
            .await
        {
            Ok(()) => {
                tracing::info!(
                    lmx_task = task,
                    lmx_kind = kind,
                    lmx_generation = generation,
                    "finalized generation {generation}"
                );
                self.set_finalize(Finalize::Idle);
                if let Err(error) = self.store.collect(Priority::Idle).await {
                    tracing::warn!(%error, "Collecting unreferenced store paths failed.");
                }
            }
            Err(reason) => {
                tracing::warn!(
                    lmx_task = task,
                    lmx_kind = kind,
                    lmx_generation = generation,
                    reason,
                    "finalizing generation {generation} failed: {reason}"
                );
                self.set_finalize(Finalize::Failed {
                    retry: Instant::now() + retry,
                    reason,
                });
            }
        }
    }

    /// Checks the booted `generation`: the mounts in process, then the health task.
    ///
    /// A failed check is logged under the check's task name, also when it failed before the task.
    async fn check(&self, generation: &str) -> Health {
        let task = self.name(Kind::SystemHealth);
        let health = match self.mounts() {
            Err(reason) => Health::Unhealthy(reason),
            Ok(()) => match self.run(&task, Kind::SystemHealth, tasks::health()).await {
                Ok(()) => Health::Healthy,
                Err(reason) => Health::Unhealthy(reason),
            },
        };
        if let Health::Unhealthy(reason) = &health {
            tracing::warn!(
                lmx_task = task,
                lmx_kind = Kind::SystemHealth.name(),
                lmx_generation = generation,
                "generation {generation} is unhealthy: {reason}"
            );
        }
        health
    }

    /// A new task name of `kind`, such as `system-health-3`.
    fn name(&self, kind: Kind) -> String {
        let number = self.created.fetch_add(1, Ordering::Relaxed) + 1;
        format!("{}-{number}", kind.task_prefix())
    }

    /// Whether the generation inputs and the development account's home are mounted.
    fn mounts(&self) -> Result<(), String> {
        let mounts = lmx_facts::mounts::shared(&self.paths.mountinfo())
            .map_err(|error| error.to_string())?;
        for target in ["/mnt/limanix", self.home.as_str()] {
            if !mounts.iter().any(|mount| mount.target == target) {
                return Err(format!("{target} is not mounted"));
            }
        }
        Ok(())
    }

    /// Runs the task `name` of `kind` once and waits; a failure gives the last line it printed, or
    /// why it ended.
    async fn run(
        &self,
        name: &str,
        kind: Kind,
        workload: ModelResult<TaskWorkload>,
    ) -> Result<(), String> {
        let mut lines = self.capture.listen(name);
        // A finalize never waits behind a build in the system slot: after the build, the older
        // generations would include the booted one. A dropped finalize is tried again later.
        let placement = match kind {
            Kind::SystemHealth => Placement {
                slot: HEALTH_SLOT,
                admission: AdmissionPolicy::Queue,
                timeout: HEALTH_TIMEOUT,
            },
            _ => Placement {
                slot: SYSTEM_SLOT,
                admission: AdmissionPolicy::DropIfRunning,
                timeout: FINALIZE_TIMEOUT,
            },
        };
        let started = match workload {
            Ok(workload) => launch::start(&self.supervisor, name, workload, placement).await,
            Err(error) => Err(error.to_string()),
        };
        let result = match started {
            Ok(task) => launch::finished(&self.supervisor, &task).await,
            Err(error) => {
                self.capture.forget(name);
                return Err(error);
            }
        };
        self.capture.forget(name);
        let mut last = None;
        while let Ok(line) = lines.try_recv() {
            if !line.text.trim().is_empty() {
                last = Some(line.text);
            }
        }
        result.map_err(|ended| last.unwrap_or(ended.message))
    }
}

/// What one look does for a settled generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Nothing: the generation is finalized and healthy, or a finalize runs or waits for its retry.
    Rest,
    /// Check health; when healthy and `finalize`, finalize.
    Check {
        /// Whether older generations wait to be removed.
        finalize: bool,
    },
}

/// What to do at `now` for a settled generation with `kept` profile generations, the last `health`
/// and `finalize`.
///
/// A failed finalize runs again once its back-off ends, even when it removed the older generations
/// before it failed, so the boot entries are rewritten.
fn step(kept: usize, health: &Health, finalize: &Finalize, now: Instant) -> Step {
    match finalize {
        Finalize::Running => Step::Rest,
        Finalize::Failed { retry, .. } if now < *retry => Step::Rest,
        Finalize::Failed { .. } => Step::Check { finalize: true },
        Finalize::Idle if kept > 1 => Step::Check { finalize: true },
        Finalize::Idle if *health == Health::Healthy => Step::Rest,
        Finalize::Idle => Step::Check { finalize: false },
    }
}

/// The generation that is desired, built and booted, if all three agree.
fn settled(generations: &Generations) -> Option<&str> {
    let desired = generations.desired.as_deref()?;
    (generations.built.as_deref() == Some(desired)
        && generations.booted.as_deref() == Some(desired))
    .then_some(desired)
}

/// Conditions of the generations, given the system profile's generation count, the last health
/// check, and where the finalize stands.
pub(crate) fn conditions(
    generations: &Generations,
    kept: Option<usize>,
    health: &Health,
    finalize: &Finalize,
) -> Vec<Condition> {
    let Some(desired) = generations.desired.as_deref() else {
        return Vec::new();
    };
    let condition = |kind: &str, message: String| Condition {
        kind: kind.to_owned(),
        message,
    };
    if generations.built.as_deref() != Some(desired) {
        return vec![condition(
            OUT_OF_DATE,
            format!("Generation {desired} is mounted but not built; apply it."),
        )];
    }
    if generations.booted.as_deref() != Some(desired) {
        return vec![condition(
            RESTART_REQUIRED,
            format!("Generation {desired} is built; restart the VM to boot it."),
        )];
    }
    match (health, finalize) {
        (Health::Unhealthy(reason), _) => vec![condition(
            DEGRADED,
            format!("Generation {desired} is booted but unhealthy: {reason}"),
        )],
        (Health::Healthy, Finalize::Failed { reason, .. }) => vec![condition(
            FINALIZE_FAILED,
            format!(
                "Finalizing generation {desired} failed: {}; lmxd tries again later.",
                reason.trim_end_matches('.')
            ),
        )],
        (Health::Healthy, Finalize::Idle) if kept == Some(1) => vec![condition(
            CONVERGED,
            format!("Generation {desired} is booted, healthy and finalized."),
        )],
        (Health::Healthy, Finalize::Idle | Finalize::Running) | (Health::Unknown, _) => Vec::new(),
    }
}

/// Looks at the generations on `schedule` until the task is dropped.
pub(crate) async fn observe(observer: Arc<Observer>, schedule: ObserverSchedule) {
    tokio::time::sleep(schedule.first).await;
    loop {
        observer.tick(schedule.retry).await;
        tokio::time::sleep(schedule.every).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generations with the three stages given.
    fn generations(desired: &str, built: &str, booted: &str) -> Generations {
        let stage = |value: &str| (!value.is_empty()).then(|| value.to_owned());
        Generations {
            desired: stage(desired),
            built: stage(built),
            booted: stage(booted),
        }
    }

    /// Kinds of the conditions for `generations`, `kept` and `health`, without a finalize.
    fn kinds(generations: &Generations, kept: Option<usize>, health: &Health) -> Vec<String> {
        conditions(generations, kept, health, &Finalize::Idle)
            .into_iter()
            .map(|condition| condition.kind)
            .collect()
    }

    #[test]
    fn follows_a_generation_from_mount_to_convergence() {
        let healthy = Health::Healthy;
        assert_eq!(
            kinds(&generations("g2", "g1", "g1"), Some(1), &healthy),
            [OUT_OF_DATE]
        );
        assert_eq!(
            kinds(&generations("g2", "g2", "g1"), Some(2), &healthy),
            [RESTART_REQUIRED]
        );
        assert!(kinds(&generations("g2", "g2", "g2"), Some(2), &healthy).is_empty());
        assert_eq!(
            kinds(&generations("g2", "g2", "g2"), Some(1), &healthy),
            [CONVERGED]
        );
    }

    #[test]
    fn an_unhealthy_booted_generation_is_degraded() {
        let health = Health::Unhealthy("sshd.service is not active".into());
        let conditions = conditions(
            &generations("g2", "g2", "g2"),
            Some(2),
            &health,
            &Finalize::Idle,
        );
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].kind, DEGRADED);
        assert!(
            conditions[0]
                .message
                .ends_with("sshd.service is not active")
        );
    }

    #[test]
    fn finalizes_when_older_generations_are_kept_and_backs_off_after_a_failure() {
        let now = Instant::now();
        let failed = |retry| Finalize::Failed {
            retry,
            reason: "boot loader update failed".into(),
        };
        let later = failed(now + Duration::from_secs(60));
        let due = failed(now);
        let healthy = Health::Healthy;
        let unhealthy = Health::Unhealthy("sshd.service is not active".into());
        let check = |finalize| Step::Check { finalize };
        assert_eq!(step(2, &Health::Unknown, &Finalize::Idle, now), check(true));
        assert_eq!(step(2, &unhealthy, &Finalize::Idle, now), check(true));
        assert_eq!(
            step(2, &healthy, &later, now),
            Step::Rest,
            "waits for the retry"
        );
        assert_eq!(
            step(1, &healthy, &due, now),
            check(true),
            "rewrites the boot entries"
        );
        assert_eq!(step(1, &healthy, &Finalize::Running, now), Step::Rest);
        assert_eq!(
            step(1, &Health::Unknown, &Finalize::Idle, now),
            check(false)
        );
        assert_eq!(step(1, &unhealthy, &Finalize::Idle, now), check(false));
        assert_eq!(
            step(1, &healthy, &Finalize::Idle, now),
            Step::Rest,
            "converged"
        );
    }

    #[test]
    fn says_nothing_without_markers_a_check_or_a_finished_finalize() {
        assert!(kinds(&generations("", "g1", "g1"), Some(1), &Health::Healthy).is_empty());
        assert!(kinds(&generations("g1", "g1", "g1"), Some(1), &Health::Unknown).is_empty());
        let settled = generations("g1", "g1", "g1");
        assert!(conditions(&settled, Some(1), &Health::Healthy, &Finalize::Running).is_empty());
    }

    #[test]
    fn a_failed_finalize_of_a_healthy_generation_is_reported() {
        let settled = generations("g1", "g1", "g1");
        let failed = Finalize::Failed {
            retry: Instant::now(),
            reason: "boot loader update failed".into(),
        };
        let conditions = conditions(&settled, Some(1), &Health::Healthy, &failed);
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].kind, FINALIZE_FAILED);
        assert!(conditions[0].message.contains("boot loader update failed"));
        let unhealthy = Health::Unhealthy("sshd.service is not active".into());
        assert_eq!(kinds_with(&settled, &unhealthy, &failed), [DEGRADED]);
        let built = generations("g2", "g2", "g1");
        assert_eq!(
            kinds_with(&built, &Health::Healthy, &failed),
            [RESTART_REQUIRED]
        );
    }

    /// Kinds of the conditions of a settled generation with one profile generation.
    fn kinds_with(generations: &Generations, health: &Health, finalize: &Finalize) -> Vec<String> {
        conditions(generations, Some(1), health, finalize)
            .into_iter()
            .map(|condition| condition.kind)
            .collect()
    }
}
