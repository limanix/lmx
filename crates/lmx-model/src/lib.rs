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
//! | [`Config`]   | NixOS, into [`CONFIG_PATH`]             | `lmx`                   |
//! | [`Envelope`] | every `lmx … --json` answer             | the LimaNix host        |
//! | [`Status`]   | `lmx status`                            | the host and people     |
//! | [`Version`]  | `lmx version`                           | the host and people     |
//!
//! JSON field names are `snake_case`. The host contract is versioned by [`CONTRACT_VERSION`] and
//! the configuration by [`CONFIG_SCHEMA`]; each changes only with a deliberate migration.
//!
//! The crate performs no I/O except [`Config::load`]. Reading the running system belongs to
//! `lmx-facts`.
#![forbid(unsafe_code)]

mod config;
mod contract;
mod status;
mod version;

pub use config::{
    CONFIG_PATH, CONFIG_SCHEMA, Config, ConfigError, DiskPolicy, Session, Tools, User, Vm,
};
pub use contract::{CONTRACT_VERSION, Envelope, ErrorBody, ErrorCode};
pub use status::{DiskUsage, Generations, Interface, Problem, Status};
pub use version::Version;
