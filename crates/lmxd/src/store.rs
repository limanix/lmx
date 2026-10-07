//! The store domain: room in the Nix store.
//!
//! It replaces three things of the platform: the store guard and its timer, the daily `nix-gc`
//! timer, and the host's reserve over SSH.
//!
//! - The guard checks the disk 5 minutes after boot and then every 15 minutes. Below the collect
//!   threshold it collects unreferenced store paths at idle priority. If the disk stays below the
//!   minimum, it lists the garbage-collector roots that keep paths alive.
//! - A reserve does the same at normal priority and answers with the usage before and after.
//! - Both share one collection: while one runs, the next caller waits for it instead of starting
//!   another.
//!
//! "Below p%" is [`DiskUsage::below`], the test the guard and the host use.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use lmx_model::{
    Condition, DISK_LOW, DiskPolicy, DiskUsage, ErrorBody, ErrorCode, Reserve, Shortage,
};
use serde_json::{Map, Value};
use solti::{
    core::SupervisorApi,
    model::{AdmissionPolicy, TaskId, TaskWorkload},
};
use tokio::sync::Mutex;

use crate::{
    launch::{self, Placement},
    tasks::{self, Kind, Priority},
};

/// Reads the usage of the store file system, or says why it cannot.
pub type UsageSource = Arc<dyn Fn() -> Result<DiskUsage, String> + Send + Sync>;

/// Slot of the store tasks; tasks in it run one at a time, in order.
const SLOT: &str = "store";

/// Longest a collection may run; Nix's garbage collection is safe to interrupt.
const COLLECT_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);

/// Longest the roots report may run.
const ROOTS_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// When the store guard checks the disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuardSchedule {
    /// Wait before the first check.
    pub first: Duration,
    /// Wait after each check before the next one.
    pub every: Duration,
}

impl GuardSchedule {
    /// The platform guard's schedule: 5 minutes after boot, then 15 minutes after each check.
    ///
    /// `uptime` is how long the system has been up; a daemon started later checks at once.
    #[must_use]
    pub fn after_boot(uptime: Duration) -> Self {
        Self {
            first: Duration::from_secs(5 * 60).saturating_sub(uptime),
            every: Duration::from_secs(15 * 60),
        }
    }
}

/// Store operations of one daemon.
pub(crate) struct Store {
    /// Supervisor that runs the store tasks.
    supervisor: Arc<SupervisorApi>,
    /// Thresholds from the platform configuration.
    policy: DiskPolicy,
    /// Reads the store disk.
    usage: UsageSource,
    /// Number of the last task created, for unique task names.
    created: AtomicU64,
    /// Name of the latest collection; held while one is found or started.
    collection: Mutex<Option<TaskId>>,
}

impl Store {
    /// Store operations that run their tasks in `supervisor`.
    pub(crate) fn new(
        supervisor: Arc<SupervisorApi>,
        policy: DiskPolicy,
        usage: UsageSource,
    ) -> Self {
        Self {
            supervisor,
            policy,
            usage,
            created: AtomicU64::new(0),
            collection: Mutex::new(None),
        }
    }

    /// Makes room for an update: collects when the disk is below the collect threshold and answers
    /// with the usage before and after.
    pub(crate) async fn reserve(&self) -> Result<Reserve, ErrorBody> {
        let before = (self.usage)().map_err(unreadable)?;
        if !before.below(self.policy.collect_percent) {
            return Ok(Reserve {
                before,
                after: before,
                freed_bytes: 0,
                collected: false,
            });
        }

        let collected = self.collect(Priority::Normal).await;
        let after = match (self.usage)() {
            Ok(after) => after,
            Err(error) => {
                if let Err(collect) = &collected {
                    tracing::warn!(error = %collect, "Collecting unreferenced store paths failed.");
                }
                return Err(unreadable(error));
            }
        };
        let freed_bytes = after.free_bytes.saturating_sub(before.free_bytes);
        let minimum = self.policy.minimum_percent;
        if after.below(minimum) {
            self.report_roots().await;
            let shortage = Shortage {
                before,
                after,
                freed_bytes,
                collect_error: collected.err(),
            };
            return Err(ErrorBody {
                code: ErrorCode::DiskLow,
                message: format!(
                    "Less than {minimum}% of the guest disk is still free after collecting \
                     unreferenced store paths."
                ),
                details: details(&shortage),
            });
        }
        if let Err(error) = &collected {
            tracing::warn!(%error, "Collecting unreferenced store paths failed.");
        }
        Ok(Reserve {
            before,
            after,
            freed_bytes,
            collected: collected.is_ok(),
        })
    }

    /// One check of the store guard.
    pub(crate) async fn guard(&self) {
        let usage = match (self.usage)() {
            Ok(usage) => usage,
            Err(error) => {
                tracing::error!(%error, "Store file-system usage cannot be read.");
                return;
            }
        };
        let collect = self.policy.collect_percent;
        if !usage.below(collect) {
            return;
        }

        tracing::info!(
            "Less than {collect}% of the guest disk is free; collecting unreferenced store paths."
        );
        if let Err(error) = self.collect(Priority::Idle).await {
            tracing::error!(%error, "Collecting unreferenced store paths failed.");
            return;
        }
        match (self.usage)() {
            Ok(usage) if usage.below(self.policy.minimum_percent) => self.report_roots().await,
            Ok(_) => {}
            Err(error) => tracing::error!(%error, "Store file-system usage cannot be read."),
        }
    }

    /// Conditions of the store disk now.
    pub(crate) fn conditions(&self) -> Vec<Condition> {
        let minimum = self.policy.minimum_percent;
        match (self.usage)() {
            Ok(usage) if usage.below(minimum) => vec![Condition {
                kind: DISK_LOW.into(),
                message: format!("Less than {minimum}% of the guest disk is free."),
            }],
            _ => Vec::new(),
        }
    }

    /// Usage of the store disk, when it is below the platform minimum or `full` says the disk ran
    /// out, so a failure can be explained as a full disk.
    pub(crate) fn shortage(&self, full: bool) -> Option<DiskUsage> {
        (self.usage)()
            .ok()
            .filter(|usage| full || usage.below(self.policy.minimum_percent))
    }

    /// Waits for the active collection, or starts one at `priority` and waits for it.
    pub(crate) async fn collect(&self, priority: Priority) -> Result<(), String> {
        let name = {
            let mut collection = self.collection.lock().await;
            match collection.as_ref().filter(|name| self.active(name)) {
                Some(name) => name.clone(),
                None => {
                    let workload = tasks::collect(priority).map_err(|error| error.to_string())?;
                    let name = self
                        .start(Kind::StoreCollect, workload, COLLECT_TIMEOUT)
                        .await?;
                    *collection = Some(name.clone());
                    name
                }
            }
        };
        self.finished(&name).await
    }

    /// Starts the roots report; its output goes to the journal, nobody waits for it.
    async fn report_roots(&self) {
        tracing::warn!(
            "Less than {}% of the guest disk is still free. Other garbage-collector roots:",
            self.policy.minimum_percent
        );
        let started = match tasks::roots() {
            Ok(workload) => self.start(Kind::StoreRoots, workload, ROOTS_TIMEOUT).await,
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = started {
            tracing::error!(%error, "Garbage-collector roots cannot be listed.");
        }
    }

    /// Creates a task of `kind` in the store slot.
    async fn start(
        &self,
        kind: Kind,
        workload: TaskWorkload,
        timeout: Duration,
    ) -> Result<TaskId, String> {
        let number = self.created.fetch_add(1, Ordering::Relaxed) + 1;
        let name = format!("{}-{number}", kind.task_prefix());
        let placement = Placement {
            slot: SLOT,
            admission: AdmissionPolicy::Queue,
            timeout,
        };
        launch::start(&self.supervisor, &name, workload, placement).await
    }

    /// Whether the task `name` will still run.
    fn active(&self, name: &TaskId) -> bool {
        self.supervisor
            .get_task(name)
            .is_some_and(|task| launch::runs(task.status()))
    }

    /// Waits until the task `name` ends; an outcome other than success is an error.
    async fn finished(&self, name: &TaskId) -> Result<(), String> {
        launch::finished(&self.supervisor, name)
            .await
            .map_err(|ended| ended.message)
    }
}

/// Error of a reserve whose disk usage cannot be read.
fn unreadable(error: String) -> ErrorBody {
    ErrorBody {
        code: ErrorCode::DiskUnreadable,
        message: format!("Store file-system usage cannot be read: {error}"),
        details: Map::new(),
    }
}

/// `shortage` as error details.
fn details(shortage: &Shortage) -> Map<String, Value> {
    match serde_json::to_value(shortage) {
        Ok(Value::Object(details)) => details,
        _ => Map::new(),
    }
}

/// Runs the store guard on `schedule` until the task is dropped.
pub(crate) async fn guard(store: Arc<Store>, schedule: GuardSchedule) {
    tokio::time::sleep(schedule.first).await;
    loop {
        store.guard().await;
        tokio::time::sleep(schedule.every).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_five_minutes_after_boot() {
        let schedule = GuardSchedule::after_boot(Duration::from_secs(60));
        assert_eq!(schedule.first, Duration::from_secs(4 * 60));
        assert_eq!(schedule.every, Duration::from_secs(15 * 60));
    }

    #[test]
    fn checks_at_once_when_started_late() {
        let schedule = GuardSchedule::after_boot(Duration::from_secs(3600));
        assert_eq!(schedule.first, Duration::ZERO);
    }

    #[test]
    fn shortage_details_name_the_collection_error_only_when_there_is_one() {
        let usage = DiskUsage {
            bytes: 100,
            free_bytes: 5,
            available_bytes: 5,
            inodes: 0,
            free_inodes: 0,
        };
        let mut shortage = Shortage {
            before: usage,
            after: usage,
            freed_bytes: 0,
            collect_error: None,
        };
        assert!(!details(&shortage).contains_key("collect_error"));
        shortage.collect_error = Some("nix-store failed".into());
        assert_eq!(details(&shortage)["collect_error"], "nix-store failed");
    }
}
