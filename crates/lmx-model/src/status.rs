//! Observed state of the guest, as reported by `lmx status`.
//!
//! Every fact is optional: a fact that cannot be read is `null` and the reason is listed in
//! [`Status::problems`]. One unreadable fact never hides the others, so the host can still show what
//! it has. Generations are the exception: a stage without a marker is `null` and is not a problem,
//! because a system built before `lmx` has none. A marker that exists but cannot be read is still a
//! problem.

use serde::{Deserialize, Serialize};

/// Observed state of the guest.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// Generations wanted by the host, built, and booted.
    pub generations: Generations,
    /// Usage of the file system that holds the Nix store.
    pub disk: Option<DiskUsage>,
    /// Network interfaces with their global IPv4 addresses.
    pub interfaces: Option<Vec<Interface>>,
    /// Names of failed systemd units.
    pub failed_units: Option<Vec<String>>,
    /// Facts or inputs that could not be read, with reasons.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<Problem>,
}

/// Generation identifiers of the three stages of an update.
///
/// The host commits the desired generation by mounting its inputs. Comparing the three values tells
/// whether a build or a restart is still needed. A stage without a known identifier, such as a system
/// built before `lmx` existed, is `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generations {
    /// Generation mounted at `/mnt/limanix`.
    pub desired: Option<String>,
    /// Generation of the current system profile.
    pub built: Option<String>,
    /// Generation the guest booted.
    pub booted: Option<String>,
}

/// Usage of one file system.
///
/// ext4 sizes its inode table with the file system, so either limit can run out first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskUsage {
    /// Total size in bytes.
    pub bytes: u64,
    /// Free bytes, including blocks reserved for root; the Nix daemon writes as root.
    pub free_bytes: u64,
    /// Free bytes available to unprivileged users.
    pub available_bytes: u64,
    /// Total inodes; zero when the file system has no fixed inode table.
    pub inodes: u64,
    /// Free inodes.
    pub free_inodes: u64,
}

/// One network interface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interface {
    /// Kernel interface name.
    pub name: String,
    /// Lowercase hardware address, when the interface has one.
    pub mac: Option<String>,
    /// Global-scope IPv4 addresses, without prefix length.
    pub ipv4: Vec<String>,
}

/// A fact or input that could not be read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// Name of the missing or incomplete [`Status`] field, or `config` when the platform
    /// configuration is unreadable.
    pub fact: String,
    /// Reason for people.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use crate::{CONTRACT_VERSION, Envelope, Status};

    /// Published examples of contract version 1 are successful answers that decode and encode
    /// without loss.
    #[test]
    fn contract_examples_round_trip() {
        for example in [
            include_str!("../../../contract/v1/status.json"),
            include_str!("../../../contract/v1/status-partial.json"),
        ] {
            let original: serde_json::Value =
                serde_json::from_str(example).expect("example is JSON");
            let envelope: Envelope<Status> =
                serde_json::from_str(example).expect("example decodes");
            assert_eq!(envelope.contract, CONTRACT_VERSION);
            assert!(envelope.ok && envelope.data.is_some() && envelope.error.is_none());
            assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);
        }
    }
}
