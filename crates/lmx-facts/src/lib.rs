//! # lmx-facts
//!
//! Readers of the running LimaNix guest.
//!
//! A fact is a value read from the system at the moment of the call: free space, booted generation,
//! interface addresses, failed units. Facts are not stored and need no daemon, so they work for any
//! caller with the caller's privileges.
//!
//! | Module          | Reads                                   | Source                                  |
//! |-----------------|-----------------------------------------|-----------------------------------------|
//! | [`disk`]        | bytes and inodes of the store           | `statvfs(3)`                            |
//! | [`generations`] | desired, built and booted generations   | generation markers in JSON files        |
//! | [`machine`]     | processors, memory and kernel           | the scheduler, `/proc/meminfo`, `uname` |
//! | [`mounts`]      | shared folders and their mode           | `/proc/self/mountinfo`                  |
//! | [`network`]     | interfaces and global IPv4 addresses    | `ip -j address show`                    |
//! | [`units`]       | failed systemd units                    | `systemctl list-units --state=failed`   |
//!
//! Readers that run a program take its path from the caller: an absolute path from the platform
//! configuration, or a `PATH` name when the configuration is unreadable. They split process I/O
//! from a pure parser, so the parsers are tested with fixed output.
#![forbid(unsafe_code)]

mod command;
pub mod disk;
mod error;
pub mod generations;
pub mod machine;
pub mod mounts;
pub mod network;
pub mod units;

pub use error::FactError;
