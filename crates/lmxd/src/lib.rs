//! # lmxd
//!
//! Guest owner daemon of a LimaNix VM. It runs as root from systemd and owns the operations that
//! must not depend on a caller's session: room in the Nix store, and updates of the system from
//! building a mounted generation to finalizing it after boot.
//!
//! | Module        | Does                                                                   |
//! |---------------|------------------------------------------------------------------------|
//! | `store`       | the store guard, reserve, and the conditions of the store disk         |
//! | `apply`       | the apply of a mounted generation and its followers                    |
//! | `observer`    | health check and finalize of the booted generation, and its conditions |
//! | `environment` | installing the environment files of a generation                       |
//! | `tasks`       | the `lmx.limanix.dev/v1` workload kinds and the runner that runs them  |
//! | `launch`      | starting tasks and waiting for them                                    |
//! | `capture`     | copies of task output for `lmxd` itself                                |
//! | `paths`       | guest locations below a system root                                    |
//! | `owner`       | the `lmx.v1.Owner` gRPC service                                        |
//! | `auth`        | caller identity from peer credentials, and who may do what             |
//! | `journal`     | task output in the daemon's log                                        |
//! | `daemon`      | startup, serving the socket, and the stopping order                    |
//!
//! `lmxd` serves two gRPC services on one Unix socket: `lmx.v1.Owner` from `lmx-ipc`, and the
//! Solti Task API, which reads and cancels the tasks behind every operation. The binary adds the
//! systemd parts: socket activation, readiness, the watchdog and the stop notification.
#![forbid(unsafe_code)]

mod apply;
mod auth;
mod capture;
mod daemon;
mod environment;
mod journal;
mod launch;
mod observer;
mod owner;
mod paths;
mod store;
mod tasks;

pub use daemon::{Daemon, Error, Options};
pub use observer::ObserverSchedule;
pub use store::{GuardSchedule, UsageSource};
pub use tasks::RegisterError;
