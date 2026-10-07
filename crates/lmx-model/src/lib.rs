//! # lmx-model
//!
//! Data contracts shared by the `lmx` guest binaries and the LimaNix host.
//!
//! The guest owner reads its platform configuration from NixOS and answers the host with JSON over
//! management SSH. Both directions are described here, so a change to either one is a change to this
//! crate.
//!
//! | Type         | Written by                              | Read by                 |
//! |--------------|-----------------------------------------|-------------------------|
//! | [`Config`]   | NixOS, into [`CONFIG_PATH`]             | `lmx` and `lmxd`        |
//! | [`Envelope`] | every `lmx … --json` answer             | the LimaNix host        |
//! | [`Status`]   | `lmx status`                            | the host and people     |
//! | [`Owner`]    | `lmxd`, through `lmx status`            | the host and people     |
//! | [`Reserve`]  | `lmx store reserve`                     | the host                |
//! | [`Apply`]    | `lmx apply`, with [`ApplyEvent`]s       | the host and people     |
//! | [`Doctor`]   | `lmx doctor`                            | the host and people     |
//! | [`NetCheck`] | `lmx net check`                         | the host and people     |
//! | [`Version`]  | `lmx version`                           | the host and people     |
//!
//! JSON field names are `snake_case`. The host contract is versioned by [`CONTRACT_VERSION`] and
//! the configuration by [`CONFIG_SCHEMA`]; each changes only with a deliberate migration.
//!
//! The crate performs no I/O except [`Config::load`]. Reading the running system belongs to
//! `lmx-facts`.
#![forbid(unsafe_code)]

mod apply;
mod check;
mod config;
mod contract;
mod owner;
mod status;
mod store;
mod version;

pub use apply::{
    Apply, ApplyEvent, ApplyPhase, ApplyState, BuildFailure, CancelApply, OutputStream,
};
pub use check::{Check, CheckStatus, Doctor, NetCheck, Protocol};
pub use config::{
    CONFIG_PATH, CONFIG_SCHEMA, Config, ConfigError, DiskPolicy, Health, Network, Ports, Session,
    Theme, Tools, User, Vm,
};
pub use contract::{CONTRACT_VERSION, Envelope, ErrorBody, ErrorCode};
pub use owner::{
    CONVERGED, Condition, DEGRADED, DISK_LOW, FINALIZE_FAILED, OUT_OF_DATE, Operation, Owner,
    RESTART_REQUIRED,
};
pub use status::{DiskUsage, Generations, Interface, Problem, Status};
pub use store::{Reserve, Shortage};
pub use version::Version;
