//! # lmx-facts
//!
//! Readers of the running LimaNix guest.
//!
//! A fact is a value read from the system at the moment of the call: free space, booted generation,
//! interface addresses, failed units.
//!
//! | Module          | Reads                                 | Source                                                  |
//! |-----------------|---------------------------------------|---------------------------------------------------------|
//! | [`disk`]        | bytes and inodes of the store         | `statvfs(3)`                                            |
//! | [`generations`] | generations and the system profile    | generation markers, the profile link                    |
//! | [`machine`]     | processors, memory, uptime and kernel | the scheduler, `/proc/meminfo`, `/proc/uptime`, `uname` |
//! | [`mounts`]      | shared folders and their mode         | `/proc/self/mountinfo`                                  |
//! | [`network`]     | interfaces and global IPv4 addresses  | `ip -j address show`                                    |
//! | [`sockets`]     | listening sockets and their processes | `/proc/net`, `/proc/<pid>/fd`                           |
//! | [`journal`]     | records of `lmxd` tasks               | `journalctl -o json`                                    |
//! | [`units`]       | failed systemd units                  | `systemctl list-units --state=failed`                   |
//!
//! Readers that run a program take its path from the caller: an absolute path from the platform
//! configuration, or a `PATH` name when the configuration is unreadable. They split process I/O
//! from a pure parser, and tests feed the parsers fixed output.
#![forbid(unsafe_code)]

mod command;
pub mod disk;
mod error;
pub mod generations;
pub mod journal;
pub mod machine;
pub mod mounts;
pub mod network;
pub mod sockets;
pub mod units;

pub use error::FactError;
