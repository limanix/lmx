//! Observed state of the guest, as reported by `lmx status`.
//!
//! Every fact is optional: a fact that cannot be read is `null` and the reason is listed in
//! [`Status::problems`]. One unreadable fact never hides the others: the host can still show what it
//! has. Generations are the exception: a stage without a marker is `null` and is not a problem,
//! because a system built before `lmx` has none. A marker that exists but cannot be read is still a
//! problem.

use serde::{Deserialize, Serialize};

use crate::Owner;

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
    /// The guest owner daemon `lmxd`, or `None` when it could not be asked.
    pub owner: Option<Owner>,
    /// Facts or inputs that could not be read, with reasons.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<Problem>,
}

/// Generation identifiers of the three stages of an update.
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
/// Either limit can run out first, because ext4 sizes its inode table with the file system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskUsage {
    /// Total size in bytes.
    pub bytes: u64,
    /// Free bytes, including blocks reserved for root.
    pub free_bytes: u64,
    /// Free bytes available to unprivileged users.
    pub available_bytes: u64,
    /// Total inodes; zero when the file system has no fixed inode table.
    pub inodes: u64,
    /// Free inodes.
    pub free_inodes: u64,
}

impl DiskUsage {
    /// Whether less than `percent` of the bytes or of the inodes is free.
    #[must_use]
    pub fn below(&self, percent: u8) -> bool {
        let low = |free: u64, total: u64| {
            total > 0 && u128::from(free) * 100 < u128::from(total) * u128::from(percent)
        };
        low(self.free_bytes, self.bytes) || low(self.free_inodes, self.inodes)
    }
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
    use crate::{CONTRACT_VERSION, DiskUsage, Envelope, ErrorCode, FINALIZE_FAILED, Status};

    fn usage(free_bytes: u64, free_inodes: u64) -> DiskUsage {
        DiskUsage {
            bytes: 1000,
            free_bytes,
            available_bytes: free_bytes,
            inodes: 1000,
            free_inodes,
        }
    }

    #[test]
    fn is_below_when_bytes_or_inodes_run_short() {
        assert!(!usage(500, 500).below(20), "healthy");
        assert!(usage(500, 150).below(20), "inodes are low");
        assert!(usage(150, 500).below(20), "bytes are low");
        assert!(
            !usage(200, 200).below(20),
            "exactly at the threshold is not below"
        );
        assert!(usage(199, 500).below(20));
    }

    #[test]
    fn never_counts_a_missing_inode_table_as_low() {
        let btrfs = DiskUsage {
            inodes: 0,
            free_inodes: 0,
            ..usage(500, 0)
        };
        assert!(!btrfs.below(20));
        assert!(!DiskUsage { bytes: 0, ..btrfs }.below(100));
    }

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

    #[test]
    fn the_failed_finalize_example_round_trips() {
        let example = include_str!("../../../contract/v1/wait-finalize-failed.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<Status> = serde_json::from_str(example).expect("example decodes");
        let error = envelope.error.clone().expect("a failure");
        assert_eq!(error.code, ErrorCode::FinalizeFailed);
        assert_eq!(error.details["conditions"][0]["type"], FINALIZE_FAILED);
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);
    }
}
