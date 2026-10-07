//! # lmxd
//!
//! Guest owner daemon of a LimaNix VM. It runs as root from systemd and owns the operations that
//! must not depend on a caller's session. In this release that is room in the Nix store.
//!
//! | Module       | Does                                                                  |
//! |--------------|-----------------------------------------------------------------------|
//! | `store`      | the store guard, reserve, and the conditions of the store disk         |
//! | `tasks`      | the `lmx.limanix.dev/v1` workload kinds and the runner that runs them |
//! | `owner`      | the `lmx.v1.Owner` gRPC service                                       |
//! | `auth`       | caller identity from peer credentials, and who may do what            |
//! | `journal`    | task output in the daemon's log                                       |
//! | `daemon`     | startup, serving the socket, and the stopping order                   |
//!
//! `lmxd` serves two gRPC services on one Unix socket: `lmx.v1.Owner` from `lmx-ipc`, and the
//! Solti Task API, which reads and cancels the tasks behind every operation. The binary adds the
//! systemd parts: socket activation, readiness, the watchdog and the stop notification.
#![forbid(unsafe_code)]

mod auth;
mod daemon;
mod journal;
mod owner;
mod store;
mod tasks;

pub use daemon::{Daemon, Error, Options};
pub use store::{GuardSchedule, UsageSource};
pub use tasks::RegisterError;
