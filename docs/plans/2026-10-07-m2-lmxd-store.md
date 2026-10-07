# M2: `lmxd` and the store domain — implementation plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `lmxd`, the root daemon on Solti that keeps room in the Nix store, and make `lmx` its client: `lmx store reserve` for the host and owner state in `lmx status`.

**Architecture:** `lmxd` runs every program as a Solti task of a kind under `lmx.limanix.dev/v1`, built by a private subprocess runner. The store guard and reserve are daemon logic that start those tasks. `lmx` and `lmxd` talk gRPC over `/run/lmx/lmx.sock` through `lmx-ipc`, which has no Solti dependency, and callers are identified by `SO_PEERCRED`.

**Tech Stack:** Rust 1.90.0 (edition 2024), Solti (local SDK by path until 0.0.7), tonic 0.14 and prost 0.14 with a vendored `protoc`, tokio, listenfd, sd-notify.

Design: [M2 design](2026-10-07-m2-lmxd-store-design.md). Background: [guest owner
design](2026-10-06-guest-owner-design.md), [S1 results](2026-10-07-s1-solti-spike.md).

## Before you start

- **Repository:** `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx`, branch `feat/m1a-foundation`, M1a and M1b committed.
- **Never commit.** The user commits; the suggested commits are at the end.
- **Solti comes from the local SDK by path.** Solti 0.0.7, with the musl fixes `lmxd` needs, is not published yet. `Cargo.toml` names `/Users/igoss/Desktop/projects/solti/sdk/crates/solti`. The containerized `task ci/*` tasks do not see that path; run the host commands below, and the Linux check at the end with the SDK mounted.
- **Commands** run on the host with the pinned toolchain. Use your own `--target-dir` if an IDE builds the repository at the same time.
- **Tests on macOS** start fake tools. The first run of a new script is slow on macOS, so the fake `nix-store` sleeps 2 seconds to make concurrent callers overlap; nothing else depends on timing.

| Task | Delivers |
|---|---|
| 1 | Contract types: `DiskUsage::below`, owner state, reserve answers, `disk.unreadable`, new tools |
| 2 | `lmx-ipc`: the `lmx.v1.Owner` protocol, conversions, a Unix-socket client |
| 3 | `lmxd`: task kinds and runner, store guard and reserve, owner service, daemon and binary |
| 4 | `lmx` as the client: `lmx store reserve`, owner state in `lmx status`, integration tests |
| 5 | Reference systemd units, `lmxd` in the release archive |
| 6 | Documentation |

---

## Task 1: Contract types for the store and the owner

**Files:**
- Create: `crates/lmx-model/src/owner.rs`, `crates/lmx-model/src/store.rs`, `contract/v1/store-reserve.json`, `contract/v1/store-reserve-disk-low.json`
- Modify: `crates/lmx-model/src/status.rs`, `crates/lmx-model/src/contract.rs`, `crates/lmx-model/src/config.rs`, `crates/lmx-model/src/lib.rs`, `contract/v1/status.json`, `contract/v1/status-partial.json`, `crates/lmx/src/status.rs`, `crates/lmx/src/help.rs`, `crates/lmx/tests/cli.rs`

Everything that crosses a process boundary lives in `lmx-model`. This task adds the values M2 sends: the threshold test,
owner state, reserve answers, a new error code and the tool paths `lmxd` runs. `lmx` only learns the new fields here;
Task 4 fills them.

**Step 1: Write the failing tests of the threshold test**

"Below p%" must decide exactly as the platform's store guard and the host do: strict, integer, bytes or inodes, and a
total of zero never counts. In `crates/lmx-model/src/status.rs`, replace the test module's first line:

```rust
    use crate::{CONTRACT_VERSION, Envelope, Status};
```

with:

```rust
    use crate::{CONTRACT_VERSION, DiskUsage, Envelope, Status};

    /// Usage with `free_bytes` of 1000 bytes and `free_inodes` of 1000 inodes.
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
```

**Step 2: Run the tests to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model`

Expected: FAIL to compile: `no method named below found for struct DiskUsage`.

**Step 3: Implement the threshold test**

In the same file, after the `DiskUsage` struct:

```rust
    /// Free inodes.
    pub free_inodes: u64,
}
```

add:

```rust
impl DiskUsage {
    /// Whether less than `percent` of the bytes or of the inodes is free.
    ///
    /// The test is strict and exact: `free × 100 < total × percent`. A total of zero, such as the
    /// inodes of a file system without an inode table, never counts as low. The platform's store
    /// guard and the LimaNix host decide the same way.
    #[must_use]
    pub fn below(&self, percent: u8) -> bool {
        let low = |free: u64, total: u64| {
            total > 0 && u128::from(free) * 100 < u128::from(total) * u128::from(percent)
        };
        low(self.free_bytes, self.bytes) || low(self.free_inodes, self.inodes)
    }
}
```

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model`

Expected: PASS (13 tests).

**Step 4: Add the owner state**

`lmx status` reports what `lmxd` is doing. Create `crates/lmx-model/src/owner.rs`:

```rust
//! State of the guest owner daemon, as `lmx status` reports it.

use serde::{Deserialize, Serialize};

/// The guest owner daemon `lmxd` and what it is doing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    /// Release version of `lmxd`.
    pub version: String,
    /// Conditions derived from facts when `lmxd` was asked; none are stored.
    pub conditions: Vec<Condition>,
    /// Operations `lmxd` has accepted and not finished.
    pub operations: Vec<Operation>,
}

/// A condition of the guest that needs attention.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    /// Stable name, such as `DiskLow`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Explanation for people.
    pub message: String,
}

/// Name of the condition set while less than the platform minimum of the store disk is free.
pub const DISK_LOW: &str = "DiskLow";

/// One operation of `lmxd`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Operation {
    /// Name of the task that runs the operation, such as `store-collect-1`.
    pub task: String,
    /// Kind of the operation, such as `StoreCollect`.
    pub kind: String,
    /// Lifecycle phase, such as `pending` or `running`.
    pub phase: String,
    /// When the operation was requested, in Unix milliseconds.
    pub created_at: u64,
}
```

In `crates/lmx-model/src/status.rs`, import it:

```rust
use serde::{Deserialize, Serialize};

/// Observed state of the guest.
```

with:

```rust
use serde::{Deserialize, Serialize};

use crate::Owner;

/// Observed state of the guest.
```

and add the field after `failed_units`. Like every other fact, it is `null` when it cannot be read:

```rust
    /// Names of failed systemd units.
    pub failed_units: Option<Vec<String>>,
```

with:

```rust
    /// Names of failed systemd units.
    pub failed_units: Option<Vec<String>>,
    /// The guest owner daemon `lmxd`, or `None` when it could not be asked.
    pub owner: Option<Owner>,
```

**Step 5: Add the reserve answers**

`lmx store reserve` answers with usage before and after; a disk that stays low is a `disk.low` failure whose details are
a `Shortage`. Create `crates/lmx-model/src/store.rs`:

```rust
//! Answers of `lmx store` commands.

use serde::{Deserialize, Serialize};

use crate::DiskUsage;

/// Answer of `lmx store reserve`: store disk usage before and after making room.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reserve {
    /// Usage before the reserve.
    pub before: DiskUsage,
    /// Usage after the reserve; equal to `before` when nothing was collected.
    pub after: DiskUsage,
    /// Free bytes gained; zero when usage grew.
    pub freed_bytes: u64,
    /// Whether unreferenced store paths were collected.
    pub collected: bool,
}

/// Details of a `disk.low` failure of `lmx store reserve`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortage {
    /// Usage before the reserve.
    pub before: DiskUsage,
    /// Usage after the reserve.
    pub after: DiskUsage,
    /// Free bytes gained; zero when usage grew.
    pub freed_bytes: u64,
    /// Why the collection failed, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collect_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::{CONTRACT_VERSION, Envelope, ErrorCode, Reserve, Shortage};

    /// The published reserve examples decode and encode without loss.
    #[test]
    fn contract_examples_round_trip() {
        let example = include_str!("../../../contract/v1/store-reserve.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<Reserve> = serde_json::from_str(example).expect("example decodes");
        assert_eq!(envelope.contract, CONTRACT_VERSION);
        assert!(envelope.ok && envelope.data.is_some());
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);

        let example = include_str!("../../../contract/v1/store-reserve-disk-low.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<Reserve> = serde_json::from_str(example).expect("example decodes");
        let error = envelope.error.clone().expect("a failure");
        assert_eq!(error.code, ErrorCode::DiskLow);
        let shortage: Shortage =
            serde_json::from_value(error.details.into()).expect("details are a shortage");
        assert!(shortage.after.below(10));
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);
    }
}
```

Its examples, `contract/v1/store-reserve.json`:

```json
{
  "contract": 1,
  "ok": true,
  "data": {
    "before": {
      "bytes": 17179869184,
      "free_bytes": 2147483648,
      "available_bytes": 1288490188,
      "inodes": 1048576,
      "free_inodes": 104857
    },
    "after": {
      "bytes": 17179869184,
      "free_bytes": 6442450944,
      "available_bytes": 5583457484,
      "inodes": 1048576,
      "free_inodes": 524288
    },
    "freed_bytes": 4294967296,
    "collected": true
  }
}
```

and `contract/v1/store-reserve-disk-low.json`:

```json
{
  "contract": 1,
  "ok": false,
  "error": {
    "code": "disk.low",
    "message": "Less than 10% of the guest disk is still free after collecting unreferenced store paths.",
    "details": {
      "before": {
        "bytes": 17179869184,
        "free_bytes": 1073741824,
        "available_bytes": 214748364,
        "inodes": 1048576,
        "free_inodes": 52428
      },
      "after": {
        "bytes": 17179869184,
        "free_bytes": 1342177280,
        "available_bytes": 483183820,
        "inodes": 1048576,
        "free_inodes": 98304
      },
      "freed_bytes": 268435456
    }
  }
}
```

The status examples gain `owner`. Replace `contract/v1/status.json`:

```json
{
  "contract": 1,
  "ok": true,
  "data": {
    "generations": {
      "desired": "0123456789ab",
      "built": "0123456789ab",
      "booted": "ba9876543210"
    },
    "disk": {
      "bytes": 17179869184,
      "free_bytes": 9663676416,
      "available_bytes": 8791261184,
      "inodes": 1048576,
      "free_inodes": 495616
    },
    "interfaces": [
      {
        "name": "lo",
        "mac": "00:00:00:00:00:00",
        "ipv4": []
      },
      {
        "name": "enp0s1",
        "mac": "52:55:55:aa:bb:cc",
        "ipv4": [
          "192.0.2.10"
        ]
      },
      {
        "name": "wg0",
        "mac": null,
        "ipv4": [
          "10.0.0.2"
        ]
      }
    ],
    "failed_units": [],
    "owner": {
      "version": "0.1.0",
      "conditions": [],
      "operations": [
        {
          "task": "store-collect-1",
          "kind": "StoreCollect",
          "phase": "running",
          "created_at": 1791374400000
        }
      ]
    }
  }
}
```

and `contract/v1/status-partial.json`, where `lmxd` was not reachable:

```json
{
  "contract": 1,
  "ok": true,
  "data": {
    "generations": {
      "desired": "0123456789ab",
      "built": null,
      "booted": null
    },
    "disk": {
      "bytes": 17179869184,
      "free_bytes": 1073741824,
      "available_bytes": 214748364,
      "inodes": 1048576,
      "free_inodes": 52428
    },
    "interfaces": null,
    "failed_units": [
      "limanix-store-guard.service"
    ],
    "owner": null,
    "problems": [
      {
        "fact": "interfaces",
        "message": "cannot run /run/current-system/sw/bin/ip: No such file or directory (os error 2)"
      },
      {
        "fact": "owner",
        "message": "lmxd is not reachable at /run/lmx/lmx.sock: No such file or directory (os error 2)"
      }
    ]
  }
}
```

Export the new modules from `crates/lmx-model/src/lib.rs`. The crate table:

```rust
//! | [`Status`]   | `lmx status`                            | the host and people     |
```

with:

```rust
//! | [`Status`]   | `lmx status`                            | the host and people     |
//! | [`Owner`]    | `lmxd`, through `lmx status`            | the host and people     |
//! | [`Reserve`]  | `lmx store reserve`                     | the host                |
```

where `lmxd` now reads the configuration too:

```rust
//! | [`Config`]   | NixOS, into [`CONFIG_PATH`]             | `lmx`                   |
```

with:

```rust
//! | [`Config`]   | NixOS, into [`CONFIG_PATH`]             | `lmx` and `lmxd`        |
```

the modules:

```rust
mod config;
mod contract;
mod status;
mod version;
```

with:

```rust
mod config;
mod contract;
mod owner;
mod status;
mod store;
mod version;
```

and the exports:

```rust
pub use contract::{CONTRACT_VERSION, Envelope, ErrorBody, ErrorCode};
pub use status::{DiskUsage, Generations, Interface, Problem, Status};
```

with:

```rust
pub use contract::{CONTRACT_VERSION, Envelope, ErrorBody, ErrorCode};
pub use owner::{Condition, DISK_LOW, Operation, Owner};
pub use status::{DiskUsage, Generations, Interface, Problem, Status};
pub use store::{Reserve, Shortage};
```

**Step 6: Add `disk.unreadable` and keep unknown codes**

`lmx` passes on the codes `lmxd` sends. A code this `lmx` does not know, such as one from a newer daemon, must still
reach the host unchanged, so `ErrorCode` gains `Other(String)` and serializes by hand. In
`crates/lmx-model/src/contract.rs`, replace the imports:

```rust
use serde::{Deserialize, Deserializer, Serialize, Serializer};
```

Replace the whole `ErrorCode` enum with:

```rust
/// Stable failure codes of the host contract.
///
/// A code this binary does not know, such as one from a newer `lmxd`, is kept as
/// [`ErrorCode::Other`], so it can be passed on unchanged and read as a generic failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// `lmxd` is not reachable.
    OwnerUnavailable,
    /// `nixos-rebuild` failed for the requested generation.
    ApplyBuildFailed,
    /// The operation was cancelled by an explicit request.
    ApplyCancelled,
    /// Free bytes or inodes are below the platform minimum.
    DiskLow,
    /// The usage of the store file system cannot be read.
    DiskUnreadable,
    /// A required network destination, such as the binary cache, is unreachable.
    NetworkUnreachable,
    /// The caller is not allowed to run the operation.
    PermissionDenied,
    /// The mounted inputs belong to a different generation than requested.
    GenerationMismatch,
    /// The caller requested a contract version this binary does not speak.
    ContractUnsupported,
    /// A code this binary does not know, as received.
    Other(String),
}

impl ErrorCode {
    /// Wire name of the code, such as `disk.low`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::OwnerUnavailable => "owner.unavailable",
            Self::ApplyBuildFailed => "apply.build_failed",
            Self::ApplyCancelled => "apply.cancelled",
            Self::DiskLow => "disk.low",
            Self::DiskUnreadable => "disk.unreadable",
            Self::NetworkUnreachable => "network.unreachable",
            Self::PermissionDenied => "permission.denied",
            Self::GenerationMismatch => "generation.mismatch",
            Self::ContractUnsupported => "contract.unsupported",
            Self::Other(code) => code,
        }
    }

    /// Code with the wire name `name`, or [`ErrorCode::Other`] for a name this binary does not know.
    #[must_use]
    pub fn from_wire(name: &str) -> Self {
        match name {
            "owner.unavailable" => Self::OwnerUnavailable,
            "apply.build_failed" => Self::ApplyBuildFailed,
            "apply.cancelled" => Self::ApplyCancelled,
            "disk.low" => Self::DiskLow,
            "disk.unreadable" => Self::DiskUnreadable,
            "network.unreachable" => Self::NetworkUnreachable,
            "permission.denied" => Self::PermissionDenied,
            "generation.mismatch" => Self::GenerationMismatch,
            "contract.unsupported" => Self::ContractUnsupported,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|name| Self::from_wire(&name))
    }
}
```

Replace the last test, `error_codes_keep_their_wire_names`, through the end of the file, with the new code and a test of
unknown codes:

```rust
    #[test]
    fn error_codes_keep_their_wire_names() {
        for (code, name) in [
            (ErrorCode::OwnerUnavailable, "owner.unavailable"),
            (ErrorCode::ApplyBuildFailed, "apply.build_failed"),
            (ErrorCode::ApplyCancelled, "apply.cancelled"),
            (ErrorCode::DiskLow, "disk.low"),
            (ErrorCode::DiskUnreadable, "disk.unreadable"),
            (ErrorCode::NetworkUnreachable, "network.unreachable"),
            (ErrorCode::PermissionDenied, "permission.denied"),
            (ErrorCode::GenerationMismatch, "generation.mismatch"),
            (ErrorCode::ContractUnsupported, "contract.unsupported"),
        ] {
            assert_eq!(serde_json::to_value(&code).expect("serialize"), name);
            assert_eq!(
                serde_json::from_value::<ErrorCode>(name.into()).expect("deserialize"),
                code
            );
        }
    }

    #[test]
    fn unknown_codes_pass_through_unchanged() {
        let code: ErrorCode = serde_json::from_value("store.busy".into()).expect("deserialize");
        assert_eq!(code, ErrorCode::Other("store.busy".into()));
        assert_eq!(
            serde_json::to_value(&code).expect("serialize"),
            "store.busy"
        );
    }
}
```

**Step 7: Add the tools `lmxd` runs**

`lmxd` runs tools with a cleared environment, and NixOS has no tools in a default `PATH` (S1), so the configuration
names each one. In `crates/lmx-model/src/config.rs`:

```rust
/// Absolute paths of the system tools the binaries run.
    pub tools: Tools,
}

/// Identity of the virtual machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vm {
    /// VM name from `limanix.toml`; also the guest host name.
    pub name: String,
    /// Guest architecture as LimaNix names it: `arm64` or `amd64`.
    pub arch: String,
    /// Human-readable operating system release, such as `NixOS 26.05`.
    pub system: String,
}

/// Development account of the guest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct User {
    /// Login name.
    pub name: String,
    /// Home directory; a host mount in LimaNix.
    pub home: String,
    /// Numeric user ID; equals the user's ID on the Mac.
    pub uid: u32,
}

/// Free-space thresholds of the guest disk, in percent of bytes or inodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiskPolicy {
    /// Below this free share, unreferenced store paths are collected.
    pub collect_percent: u8,
    /// Below this free share, builds may fail and LimaNix warns.
    pub minimum_percent: u8,
}

/// Named-session provider selected by catalog modules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    /// Absolute provider executable that receives one session name, if a module selected one.
    pub command: Option<String>,
    /// Catalog selectors suggested when no provider is selected.
    pub providers: Vec<String>,
}

/// Absolute paths of the system tools the binaries run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    /// `ip` from iproute2, used for interface addresses.
    pub ip: String,
    /// `systemctl`, used for failed units.
    pub systemctl: String,
}
```

with:

```rust
/// Absolute paths of the system tools the binaries run.
    pub tools: Tools,
}

/// Identity of the virtual machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vm {
    /// VM name from `limanix.toml`; also the guest host name.
    pub name: String,
    /// Guest architecture as LimaNix names it: `arm64` or `amd64`.
    pub arch: String,
    /// Human-readable operating system release, such as `NixOS 26.05`.
    pub system: String,
}

/// Development account of the guest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct User {
    /// Login name.
    pub name: String,
    /// Home directory; a host mount in LimaNix.
    pub home: String,
    /// Numeric user ID; equals the user's ID on the Mac.
    pub uid: u32,
}

/// Free-space thresholds of the guest disk, in percent of bytes or inodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiskPolicy {
    /// Below this free share, unreferenced store paths are collected.
    pub collect_percent: u8,
    /// Below this free share, builds may fail and LimaNix warns.
    pub minimum_percent: u8,
}

/// Named-session provider selected by catalog modules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    /// Absolute provider executable that receives one session name, if a module selected one.
    pub command: Option<String>,
    /// Catalog selectors suggested when no provider is selected.
    pub providers: Vec<String>,
}

/// Absolute paths of the system tools the binaries run.
///
/// `lmxd` runs its tools with a cleared environment, and NixOS has no tools in a default `PATH`, so
/// every tool is named by its store path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    /// `ip` from iproute2, used for interface addresses.
    pub ip: String,
    /// `systemctl`, used for failed units.
    pub systemctl: String,
    /// `nix-store`, used to collect unreferenced store paths and to list garbage-collector roots.
    pub nix_store: String,
    /// `nice` from coreutils, used to run the guard's collections at the lowest CPU priority.
    pub nice: String,
    /// `ionice` from util-linux, used to run the guard's collections in the idle I/O class.
    pub ionice: String,
    /// `grep`, used to leave the expected roots out of the garbage-collector roots report.
    pub grep: String,
}
```

and in its test sample:

```rust
        "tools": {"ip": "/run/current-system/sw/bin/ip", "systemctl": "/run/current-system/sw/bin/systemctl"}
```

with:

```rust
        "tools": {
            "ip": "/run/current-system/sw/bin/ip",
            "systemctl": "/run/current-system/sw/bin/systemctl",
            "nix_store": "/run/current-system/sw/bin/nix-store",
            "nice": "/run/current-system/sw/bin/nice",
            "ionice": "/run/current-system/sw/bin/ionice",
            "grep": "/run/current-system/sw/bin/grep"
        }
```

**Step 8: Keep `lmx` building**

`lmx` builds `Status` and `Tools` itself. Until Task 4 asks `lmxd`, it reports no owner. In `crates/lmx/src/status.rs`,
in `collect`:

```rust
            units::failed(&system.systemctl()),
            &mut problems,
        ),
        problems,
```

with:

```rust
            units::failed(&system.systemctl()),
            &mut problems,
        ),
        owner: None,
        problems,
```

and in the test `renders_every_fact_on_its_own_line`:

```rust
            failed_units: Some(vec![]),
            problems: vec![],
```

with:

```rust
            failed_units: Some(vec![]),
            owner: None,
            problems: vec![],
```

In `crates/lmx/src/help.rs`, the test configuration:

```rust
            tools: Tools {
                ip: "ip".into(),
                systemctl: "systemctl".into(),
            },
```

with:

```rust
            tools: Tools {
                ip: "ip".into(),
                systemctl: "systemctl".into(),
                nix_store: "nix-store".into(),
                nice: "nice".into(),
                ionice: "ionice".into(),
                grep: "grep".into(),
            },
```

In `crates/lmx/tests/cli.rs`, the guest configuration, which is rejected without the new tools:

```rust
            "tools": {"ip": tools.ip, "systemctl": tools.systemctl}
```

with:

```rust
            "tools": {
                "ip": tools.ip,
                "systemctl": tools.systemctl,
                "nix_store": "/run/current-system/sw/bin/nix-store",
                "nice": "/run/current-system/sw/bin/nice",
                "ionice": "/run/current-system/sw/bin/ionice",
                "grep": "/run/current-system/sw/bin/grep"
            }
```

**Step 9: Run the workspace tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml --workspace`

Expected: PASS (104 tests), including the round trips of the new examples.

---

## Task 2: `lmx-ipc`, the protocol between `lmx` and `lmxd`

**Files:**
- Create: `crates/lmx-ipc/Cargo.toml`, `crates/lmx-ipc/build.rs`, `crates/lmx-ipc/proto/lmx/v1/owner.proto`, `crates/lmx-ipc/src/lib.rs`, `crates/lmx-ipc/src/client.rs`, `crates/lmx-ipc/src/convert.rs`
- Modify: `Cargo.toml`

Both binaries share this crate. It has no Solti dependency, so `lmx` stays light: only `lmxd` links the daemon runtime.

**Step 1: Add the crate and its dependencies to the workspace**

In `Cargo.toml`, the members:

```toml
members  = ["crates/lmx", "crates/lmx-facts", "crates/lmx-model"]
```

with:

```toml
members  = ["crates/lmx", "crates/lmx-facts", "crates/lmx-ipc", "crates/lmx-model"]
```

and the whole `[workspace.dependencies]` table. `solti` and the daemon crates are listed now and used in Task 3:

```toml
[workspace.dependencies]
lmx-facts           = { path = "crates/lmx-facts" }
lmx-ipc             = { path = "crates/lmx-ipc" }
lmx-model           = { path = "crates/lmx-model" }
base64              = "0.23.1"
clap                = { version = "4.6.6", features = ["derive"] }
hyper-util          = { version = "0.1.20", features = ["tokio"] }
listenfd            = "1.0.2"
prost               = "0.14.4"
protoc-bin-vendored = "3.2.0"
rustix              = { version = "1.1.4", features = ["fs", "system"] }
sd-notify           = "0.4.5"
serde               = { version = "1.0.229", features = ["derive"] }
serde_json          = "1.0.151"
# Solti 0.0.7 is not published yet: it carries the musl fixes `lmxd` needs (S1). Switch to the
# crates.io release once it is out.
solti               = { path = "/Users/igoss/Desktop/projects/solti/sdk/crates/solti", default-features = false }
tempfile            = "3.27.0"
thiserror           = "2.0.20"
tokio               = "1.53.1"
tokio-stream        = { version = "0.1.19", features = ["net"] }
tonic               = "0.14.6"
tonic-prost         = "0.14.6"
tonic-prost-build   = "0.14.6"
tower               = { version = "0.5.3", features = ["util"] }
tracing             = "0.1.44"
unicode-width       = "0.2.2"
```

**Step 2: Create the crate**

`crates/lmx-ipc/Cargo.toml`:

```toml
[package]
name         = "lmx-ipc"
description  = "Calls between lmx and the guest owner daemon lmxd"
version.workspace      = true
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
repository.workspace   = true
homepage.workspace     = true
publish = false

[dependencies]
hyper-util  = { workspace = true }
lmx-model   = { workspace = true }
prost       = { workspace = true }
serde_json  = { workspace = true }
tokio       = { workspace = true, features = ["net"] }
tonic       = { workspace = true }
tonic-prost = { workspace = true }
tower       = { workspace = true }

[build-dependencies]
protoc-bin-vendored = { workspace = true }
tonic-prost-build   = { workspace = true }

[lints]
workspace = true
```

`crates/lmx-ipc/build.rs` generates the code with a vendored `protoc`, so no build host needs one:

```rust
//! Generates the gRPC code of `lmx.v1` with a vendored `protoc`, so builds need no system protoc.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = tonic_prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    tonic_prost_build::configure().compile_with_config(
        config,
        &["proto/lmx/v1/owner.proto"],
        &["proto"],
    )?;
    Ok(())
}
```

`crates/lmx-ipc/proto/lmx/v1/owner.proto`. Domain failures travel in the answer with contract codes; a gRPC status means
a transport failure:

```protobuf
syntax = "proto3";

// Calls from `lmx` to the guest owner daemon `lmxd`, served on `/run/lmx/lmx.sock`.
package lmx.v1;

// The guest owner daemon.
service Owner {
  // What `lmxd` is doing and which conditions hold. Open to every caller.
  rpc Status(StatusRequest) returns (StatusResponse);
  // Collects unreferenced store paths when free space is low. Root only.
  rpc Reserve(ReserveRequest) returns (ReserveResponse);
}

// Usage of one file system, as in the host contract.
message DiskUsage {
  // Total size in bytes.
  uint64 bytes = 1;
  // Free bytes, including blocks reserved for root.
  uint64 free_bytes = 2;
  // Free bytes available to unprivileged users.
  uint64 available_bytes = 3;
  // Total inodes; zero when the file system has no fixed inode table.
  uint64 inodes = 4;
  // Free inodes.
  uint64 free_inodes = 5;
}

// A failure the host contract names.
message Failure {
  // Host contract error code, such as `disk.low`.
  string code = 1;
  // Explanation for people.
  string message = 2;
  // Code-specific details as a JSON object; empty without details.
  string details = 3;
}

// Input of `Status`.
message StatusRequest {}

// A condition of the guest that needs attention.
message Condition {
  // Stable name, such as `DiskLow`.
  string type = 1;
  // Explanation for people.
  string message = 2;
}

// One operation of `lmxd`.
message Operation {
  // Name of the task that runs the operation, such as `store-collect-1`.
  string task = 1;
  // Kind of the operation, such as `StoreCollect`.
  string kind = 2;
  // Lifecycle phase, such as `running`.
  string phase = 3;
  // When the operation was requested, in Unix milliseconds.
  uint64 created_at = 4;
}

// Output of `Status`.
message StatusResponse {
  // Release version of `lmxd`.
  string version = 1;
  // Conditions derived from facts when `lmxd` was asked.
  repeated Condition conditions = 2;
  // Operations `lmxd` has accepted and not finished.
  repeated Operation operations = 3;
}

// Input of `Reserve`.
message ReserveRequest {}

// Store disk usage before and after a reserve.
message ReserveResult {
  // Usage before the reserve.
  DiskUsage before = 1;
  // Usage after the reserve.
  DiskUsage after = 2;
  // Free bytes gained; zero when usage grew.
  uint64 freed_bytes = 3;
  // Whether unreferenced store paths were collected.
  bool collected = 4;
}

// Output of `Reserve`.
message ReserveResponse {
  // A result, or a failure such as `disk.low`.
  oneof outcome {
    // The reserve succeeded.
    ReserveResult result = 1;
    // The reserve failed.
    Failure failure = 2;
  }
}
```

`crates/lmx-ipc/src/lib.rs`:

```rust
//! # lmx-ipc
//!
//! Calls between `lmx` and the guest owner daemon `lmxd`.
//!
//! `lmxd` serves gRPC on the Unix socket [`SOCKET_PATH`]. This crate holds what both sides share:
//!
//! | Item          | Is                                                                    |
//! |---------------|-----------------------------------------------------------------------|
//! | [`proto`]     | the `lmx.v1.Owner` service, generated from `proto/lmx/v1/owner.proto` |
//! | [`connect`]   | a client of that service over the socket                              |
//! | `From` impls  | conversions between its messages and the `lmx-model` contract types   |
//!
//! The socket is open to every local user; `lmxd` decides what each caller may do from the peer
//! credentials of the connection. The crate has no Solti dependency, so `lmx` stays light.
#![forbid(unsafe_code)]

mod client;
mod convert;

pub use client::connect;
pub use convert::{InvalidAnswer, reserve_outcome};
pub use tonic;

/// Path of the socket inside a booted guest.
pub const SOCKET_PATH: &str = "/run/lmx/lmx.sock";

/// Messages and services of `lmx.v1`, generated from `proto/lmx/v1/owner.proto`.
#[allow(
    missing_docs,
    unreachable_pub,
    clippy::missing_docs_in_private_items,
    clippy::doc_markdown
)]
pub mod proto {
    tonic::include_proto!("lmx.v1");
}
```

`crates/lmx-ipc/src/client.rs` connects the socket first, so a missing daemon is reported with the operating system's
reason:

```rust
//! Connection to `lmxd` over its Unix socket.

use std::{io, path::Path};

use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use crate::proto::owner_client::OwnerClient;

/// Connects to `lmxd` on the socket at `path`.
///
/// The socket is connected before the client is built, so a missing socket or a refused connection
/// is reported with the operating system's reason. A socket that systemd holds for a daemon that does
/// not run yet accepts the connection, so callers bound their first call. The connection is used
/// once: a client that loses it fails its next call instead of reconnecting.
pub async fn connect(path: &Path) -> io::Result<OwnerClient<Channel>> {
    let mut stream = Some(UnixStream::connect(path).await?);
    // The URI only names the peer in HTTP/2 requests; the connector ignores it.
    let channel = Endpoint::from_static("http://lmxd")
        .connect_with_connector(service_fn(move |_: Uri| {
            let stream = stream.take();
            async move {
                stream.map(TokioIo::new).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotConnected, "the lmxd connection was lost")
                })
            }
        }))
        .await
        .map_err(io::Error::other)?;
    Ok(OwnerClient::new(channel))
}
```

`crates/lmx-ipc/src/convert.rs` maps the messages to the contract types and back:

```rust
//! Conversions between the `lmx.v1` messages and the `lmx-model` contract types.

use lmx_model::{Condition, DiskUsage, ErrorBody, ErrorCode, Operation, Owner, Reserve};
use serde_json::{Map, Value};

use crate::proto;

impl From<DiskUsage> for proto::DiskUsage {
    fn from(usage: DiskUsage) -> Self {
        Self {
            bytes: usage.bytes,
            free_bytes: usage.free_bytes,
            available_bytes: usage.available_bytes,
            inodes: usage.inodes,
            free_inodes: usage.free_inodes,
        }
    }
}

impl From<proto::DiskUsage> for DiskUsage {
    fn from(usage: proto::DiskUsage) -> Self {
        Self {
            bytes: usage.bytes,
            free_bytes: usage.free_bytes,
            available_bytes: usage.available_bytes,
            inodes: usage.inodes,
            free_inodes: usage.free_inodes,
        }
    }
}

impl From<Owner> for proto::StatusResponse {
    fn from(owner: Owner) -> Self {
        Self {
            version: owner.version,
            conditions: owner
                .conditions
                .into_iter()
                .map(|condition| proto::Condition {
                    r#type: condition.kind,
                    message: condition.message,
                })
                .collect(),
            operations: owner
                .operations
                .into_iter()
                .map(|operation| proto::Operation {
                    task: operation.task,
                    kind: operation.kind,
                    phase: operation.phase,
                    created_at: operation.created_at,
                })
                .collect(),
        }
    }
}

impl From<proto::StatusResponse> for Owner {
    fn from(response: proto::StatusResponse) -> Self {
        Self {
            version: response.version,
            conditions: response
                .conditions
                .into_iter()
                .map(|condition| Condition {
                    kind: condition.r#type,
                    message: condition.message,
                })
                .collect(),
            operations: response
                .operations
                .into_iter()
                .map(|operation| Operation {
                    task: operation.task,
                    kind: operation.kind,
                    phase: operation.phase,
                    created_at: operation.created_at,
                })
                .collect(),
        }
    }
}

impl From<Reserve> for proto::ReserveResult {
    fn from(reserve: Reserve) -> Self {
        Self {
            before: Some(reserve.before.into()),
            after: Some(reserve.after.into()),
            freed_bytes: reserve.freed_bytes,
            collected: reserve.collected,
        }
    }
}

impl From<ErrorBody> for proto::Failure {
    fn from(error: ErrorBody) -> Self {
        Self {
            code: error.code.as_str().to_owned(),
            message: error.message,
            details: if error.details.is_empty() {
                String::new()
            } else {
                Value::Object(error.details).to_string()
            },
        }
    }
}

impl From<proto::Failure> for ErrorBody {
    fn from(failure: proto::Failure) -> Self {
        let details = match serde_json::from_str(&failure.details) {
            Ok(Value::Object(details)) => details,
            _ => Map::new(),
        };
        Self {
            code: ErrorCode::from_wire(&failure.code),
            message: failure.message,
            details,
        }
    }
}

/// An answer of `lmxd` that breaks the `lmx.v1` protocol, such as one without an outcome.
#[derive(Debug, PartialEq, Eq)]
pub struct InvalidAnswer(pub &'static str);

impl std::fmt::Display for InvalidAnswer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "lmxd answered without {}", self.0)
    }
}

impl std::error::Error for InvalidAnswer {}

/// Outcome of a `Reserve` call: the result, or the failure the contract names.
pub fn reserve_outcome(
    response: proto::ReserveResponse,
) -> Result<Result<Reserve, ErrorBody>, InvalidAnswer> {
    match response.outcome {
        Some(proto::reserve_response::Outcome::Result(result)) => Ok(Ok(Reserve {
            before: result.before.ok_or(InvalidAnswer("usage before"))?.into(),
            after: result.after.ok_or(InvalidAnswer("usage after"))?.into(),
            freed_bytes: result.freed_bytes,
            collected: result.collected,
        })),
        Some(proto::reserve_response::Outcome::Failure(failure)) => Ok(Err(failure.into())),
        None => Err(InvalidAnswer("an outcome")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Usage with distinct values in every field.
    const USAGE: DiskUsage = DiskUsage {
        bytes: 1,
        free_bytes: 2,
        available_bytes: 3,
        inodes: 4,
        free_inodes: 5,
    };

    #[test]
    fn owner_state_survives_the_wire() {
        let owner = Owner {
            version: "0.1.0".into(),
            conditions: vec![Condition {
                kind: "DiskLow".into(),
                message: "Less than 10% of the guest disk is free.".into(),
            }],
            operations: vec![Operation {
                task: "store-collect-1".into(),
                kind: "StoreCollect".into(),
                phase: "running".into(),
                created_at: 1_791_374_400_000,
            }],
        };
        assert_eq!(
            Owner::from(proto::StatusResponse::from(owner.clone())),
            owner
        );
    }

    #[test]
    fn reserve_results_and_failures_survive_the_wire() {
        let reserve = Reserve {
            before: USAGE,
            after: USAGE,
            freed_bytes: 7,
            collected: true,
        };
        let response = proto::ReserveResponse {
            outcome: Some(proto::reserve_response::Outcome::Result(
                reserve.clone().into(),
            )),
        };
        assert_eq!(reserve_outcome(response), Ok(Ok(reserve)));

        let mut details = Map::new();
        details.insert("freed_bytes".into(), 7.into());
        let error = ErrorBody {
            code: ErrorCode::DiskLow,
            message: "full".into(),
            details,
        };
        let response = proto::ReserveResponse {
            outcome: Some(proto::reserve_response::Outcome::Failure(
                error.clone().into(),
            )),
        };
        assert_eq!(reserve_outcome(response), Ok(Err(error)));
    }

    #[test]
    fn rejects_an_answer_without_an_outcome() {
        let response = proto::ReserveResponse { outcome: None };
        assert_eq!(
            reserve_outcome(response).map_err(|error| error.to_string()),
            Err("lmxd answered without an outcome".into())
        );
    }
}
```

**Step 3: Run the crate's tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-ipc`

Expected: PASS (3 tests): owner state and reserve outcomes survive the wire.

---

## Task 3: `lmxd`, the guest owner daemon

**Files:**
- Create: `crates/lmxd/Cargo.toml`, `crates/lmxd/src/lib.rs`, `crates/lmxd/src/tasks.rs`, `crates/lmxd/src/store.rs`, `crates/lmxd/src/auth.rs`, `crates/lmxd/src/journal.rs`, `crates/lmxd/src/owner.rs`, `crates/lmxd/src/daemon.rs`, `crates/lmxd/src/main.rs`
- Modify: `Cargo.toml`

The daemon is a library plus a binary. The library starts the supervisor and serves a socket; tests start it in process
with a scripted disk. The binary adds systemd: socket activation, readiness, the watchdog and the stop notification.

**Step 1: Add the crate to the workspace**

```toml
members  = ["crates/lmx", "crates/lmx-facts", "crates/lmx-ipc", "crates/lmx-model"]
```

with:

```toml
members  = ["crates/lmx", "crates/lmx-facts", "crates/lmx-ipc", "crates/lmx-model", "crates/lmxd"]
```

and the path dependency, used by `lmx`'s tests in Task 4:

```toml
lmx-model           = { path = "crates/lmx-model" }
```

with:

```toml
lmx-model           = { path = "crates/lmx-model" }
lmxd                = { path = "crates/lmxd" }
```

**Step 2: Create the crate**

`crates/lmxd/Cargo.toml`:

```rust
[package]
name         = "lmxd"
description  = "Guest owner daemon of a LimaNix VM"
version.workspace      = true
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
repository.workspace   = true
homepage.workspace     = true
publish = false

[dependencies]
clap         = { workspace = true }
lmx-facts    = { workspace = true }
lmx-ipc      = { workspace = true }
lmx-model    = { workspace = true }
listenfd     = { workspace = true }
rustix       = { workspace = true, features = ["process"] }
sd-notify    = { workspace = true }
serde_json   = { workspace = true }
solti        = { workspace = true, features = ["api-core-adapter", "api-grpc", "core", "exec-subprocess", "observe-journald"] }
thiserror    = { workspace = true }
tokio        = { workspace = true, features = ["macros", "net", "rt-multi-thread", "signal", "sync", "time"] }
tokio-stream = { workspace = true }
tonic        = { workspace = true }
tracing      = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }

[lints]
workspace = true
```

`crates/lmxd/src/tasks.rs` defines the kinds and the runner. The runner turns an `lmxd` task into a subprocess task and
builds it with a private subprocess runner, as `solti-chain` does; the private runner is not registered with the
supervisor, so no API caller can start an arbitrary program as root:

```rust
//! Workload kinds of `lmxd` and the runner that executes them.
//!
//! Every operation of `lmxd` is a Solti task whose kind lives under [`API_VERSION`], so the Task API
//! lists it, streams its output and keeps its runs. Each kind runs one program. The runner turns
//! such a task into a `solti.io/v1` subprocess task and builds it with a private subprocess runner,
//! which clears the environment, owns the process group and captures the output. The private runner
//! is not registered with the supervisor, so no API caller can start an arbitrary program as root.

use std::sync::Arc;

use lmx_model::Tools;
use serde_json::{Value, json};
use solti::{
    exec::{
        ExecError,
        subprocess::{SubprocessRunner, register_subprocess_runner},
    },
    model::{
        ExtensionWorkload, Flag, ModelResult, SubprocessMode, SubprocessSpec, Task, TaskEnv,
        TaskWorkload, WorkloadTypeMeta,
    },
    runner::{
        BuildCancellation, BuildContext, BuildScope, RouterError, RunId, Runner, RunnerCatalog,
        RunnerError, RunnerRouter, async_trait,
    },
    taskvisor::TaskRef,
};

/// API version of the workload kinds of `lmxd`.
pub(crate) const API_VERSION: &str = "lmx.limanix.dev/v1";

/// Shell script that lists garbage-collector roots, leaving out those that always exist: running
/// processes, runtime state and the system profile. `$1` is `nix-store` and `$2` is `grep`. Its exit
/// status is ignored, as the platform's store guard ignored it.
const ROOTS_SCRIPT: &str = r#""$1" --gc --print-roots | "$2" -E -v -e '^"?/proc/' -e '^"?/run/' -e '^"?/nix/var/nix/profiles/system' -e '[{]censored[}]'
exit 0"#;

/// Workload kinds of `lmxd`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Collects unreferenced store paths with `nix-store --gc`.
    StoreCollect,
    /// Prints the garbage-collector roots that keep store paths alive.
    StoreRoots,
}

impl Kind {
    /// Every kind, in declaration order.
    const ALL: [Self; 2] = [Self::StoreCollect, Self::StoreRoots];

    /// Kind name under [`API_VERSION`].
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::StoreCollect => "StoreCollect",
            Self::StoreRoots => "StoreRoots",
        }
    }

    /// Prefix of the names of this kind's tasks; a counter follows it.
    pub(crate) const fn task_prefix(self) -> &'static str {
        match self {
            Self::StoreCollect => "store-collect",
            Self::StoreRoots => "store-roots",
        }
    }

    /// Kind with the name `name`.
    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// CPU and I/O priority of a collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Priority {
    /// Normal priority; reserve uses it, because the host waits for the result.
    Normal,
    /// The lowest CPU priority and the idle I/O class, as the platform's store guard ran.
    Idle,
}

impl Priority {
    /// Spec value of the priority.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Idle => "idle",
        }
    }
}

/// Workload of a `StoreCollect` task.
pub(crate) fn collect(priority: Priority) -> ModelResult<TaskWorkload> {
    extension(Kind::StoreCollect, json!({"priority": priority.as_str()}))
}

/// Workload of a `StoreRoots` task.
pub(crate) fn roots() -> ModelResult<TaskWorkload> {
    extension(Kind::StoreRoots, json!({}))
}

/// Workload of `kind` with `spec`.
fn extension(kind: Kind, spec: Value) -> ModelResult<TaskWorkload> {
    ExtensionWorkload::new(API_VERSION, kind.name(), spec).map(TaskWorkload::Extension)
}

/// Program and arguments that run a task of `kind` with `spec`.
fn command(kind: Kind, spec: &Value, tools: &Tools) -> (String, Vec<String>) {
    match kind {
        Kind::StoreCollect => {
            let collect = [tools.nix_store.clone(), "--gc".into(), "--quiet".into()];
            if spec.get("priority").and_then(Value::as_str) == Some(Priority::Idle.as_str()) {
                let mut args = vec!["-n".into(), "19".into(), tools.ionice.clone()];
                args.extend(["-c".into(), "3".into()]);
                args.extend(collect);
                (tools.nice.clone(), args)
            } else {
                let [program, args @ ..] = collect;
                (program, args.to_vec())
            }
        }
        Kind::StoreRoots => (
            "/bin/sh".into(),
            vec![
                "-c".into(),
                ROOTS_SCRIPT.into(),
                "sh".into(),
                tools.nix_store.clone(),
                tools.grep.clone(),
            ],
        ),
    }
}

/// Runner of the `lmxd` kinds.
struct LmxRunner {
    /// Catalog of the private subprocess runner.
    catalog: RunnerCatalog,
    /// Absolute paths of the programs the kinds run.
    tools: Tools,
}

#[async_trait]
impl Runner for LmxRunner {
    fn name(&self) -> &str {
        "lmx"
    }

    fn workload_types(&self) -> Vec<WorkloadTypeMeta> {
        Kind::ALL
            .into_iter()
            .map(|kind| WorkloadTypeMeta::new(API_VERSION, kind.name()).expect("valid kind"))
            .collect()
    }

    async fn build_task(
        &self,
        task: &Task,
        _run_id: &RunId,
        ctx: &BuildContext,
        cancellation: &BuildCancellation,
        scope: &mut BuildScope,
    ) -> Result<TaskRef, RunnerError> {
        let TaskWorkload::Extension(workload) = task.spec().workload() else {
            return Err(RunnerError::InvalidSpec("not an lmxd workload".into()));
        };
        let kind = Kind::from_name(workload.kind())
            .ok_or_else(|| RunnerError::InvalidSpec(format!("unknown kind {}", workload.kind())))?;
        let (command, args) = command(kind, workload.spec(), &self.tools);
        let process = TaskWorkload::Subprocess(SubprocessSpec::new(
            SubprocessMode::Command { command, args },
            TaskEnv::new(),
            None,
            Flag::enabled(),
        ));
        // Same name, generation and status, so output and runs belong to the `lmxd` task.
        let derived = Task::from_parts(
            task.type_meta().clone(),
            task.metadata().clone(),
            task.spec()
                .derive_with_workload(process)
                .without_runner_selector(),
            task.status().clone(),
        )
        .map_err(|error| RunnerError::InvalidSpec(error.to_string()))?;
        let built = self
            .catalog
            .build_scoped_with_cancellation(&derived, ctx, cancellation, scope)
            .await
            .map_err(|source| RunnerError::NestedBuild {
                context: format!("{} task {}", kind.name(), task.name()),
                source: Box::new(source),
            })?;
        Ok(built.into_task())
    }
}

/// Failure to register the runners.
#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    /// The subprocess runner could not be created.
    #[error("cannot create the subprocess runner: {0}")]
    Subprocess(#[from] ExecError),
    /// A runner was rejected by its router.
    #[error("cannot register a runner: {0}")]
    Router(#[from] RouterError),
}

/// Registers the runner of the `lmxd` kinds in `router`.
///
/// Returns the private subprocess runner, which must be shut down after the supervisor.
pub(crate) fn register(
    router: &mut RunnerRouter,
    tools: Tools,
) -> Result<Arc<SubprocessRunner>, RegisterError> {
    let mut private = RunnerRouter::new();
    let subprocess = register_subprocess_runner(&mut private, "lmx-exec")?;
    router.register(Arc::new(LmxRunner {
        catalog: private.catalog(),
        tools,
    }))?;
    Ok(subprocess)
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

    use super::*;

    /// Tools with recognizable paths.
    fn tools() -> Tools {
        Tools {
            ip: "/bin/ip".into(),
            systemctl: "/bin/systemctl".into(),
            nix_store: "/nix/bin/nix-store".into(),
            nice: "/bin/nice".into(),
            ionice: "/bin/ionice".into(),
            grep: "/bin/grep".into(),
        }
    }

    #[test]
    fn collects_at_normal_priority() {
        let (program, args) = command(Kind::StoreCollect, &json!({"priority": "normal"}), &tools());
        assert_eq!(program, "/nix/bin/nix-store");
        assert_eq!(args, ["--gc", "--quiet"]);
    }

    #[test]
    fn collects_for_the_guard_at_idle_priority() {
        let (program, args) = command(Kind::StoreCollect, &json!({"priority": "idle"}), &tools());
        assert_eq!(program, "/bin/nice");
        assert_eq!(
            args,
            [
                "-n",
                "19",
                "/bin/ionice",
                "-c",
                "3",
                "/nix/bin/nix-store",
                "--gc",
                "--quiet"
            ]
        );
    }

    #[test]
    fn lists_roots_through_the_configured_tools() {
        let (program, args) = command(Kind::StoreRoots, &json!({}), &tools());
        assert_eq!(program, "/bin/sh");
        assert_eq!(args[..2], ["-c".to_owned(), ROOTS_SCRIPT.to_owned()]);
        assert_eq!(args[2..], ["sh", "/nix/bin/nix-store", "/bin/grep"]);
    }

    #[test]
    fn the_roots_report_leaves_out_the_roots_that_always_exist() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let nix_store = directory.path().join("nix-store");
        fs::write(
            &nix_store,
            r#"#!/bin/sh
printf '%s\n' \
  '"/proc/1/maps" -> /nix/store/aaaa-glibc' \
  '/proc/2/environ -> /nix/store/bbbb-bash' \
  '/run/booted-system -> /nix/store/cccc-system' \
  '/nix/var/nix/profiles/system-42-link -> /nix/store/dddd-system' \
  '{censored} -> /nix/store/eeee-secret' \
  '/home/dev/project/.direnv/flake-inputs/ffff-source -> /nix/store/ffff-source'
"#,
        )
        .expect("write nix-store");
        fs::set_permissions(&nix_store, fs::Permissions::from_mode(0o755))
            .expect("make nix-store executable");
        let grep = ["/usr/bin/grep", "/bin/grep"]
            .into_iter()
            .find(|path| Path::new(path).exists())
            .expect("grep is installed");

        let output = Command::new("/bin/sh")
            .args(["-c", ROOTS_SCRIPT, "sh"])
            .arg(&nix_store)
            .arg(grep)
            .output()
            .expect("run the roots report");
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stderr), "");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "/home/dev/project/.direnv/flake-inputs/ffff-source -> /nix/store/ffff-source\n"
        );
    }

    #[test]
    fn names_every_kind_under_the_lmx_api_version() {
        for kind in Kind::ALL {
            assert_eq!(Kind::from_name(kind.name()), Some(kind));
            assert!(WorkloadTypeMeta::new(API_VERSION, kind.name()).is_ok());
        }
    }
}
```

`crates/lmxd/src/store.rs` holds the guard, reserve and conditions. One lock finds or starts the collection, so
concurrent callers share it:

```rust
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
    model::{
        AdmissionPolicy, RestartPolicy, TaskId, TaskManifest, TaskPhase, TaskSpec, TaskStatus,
        TaskWorkload,
    },
};
use tokio::sync::Mutex;

use crate::tasks::{self, Kind, Priority};

/// Reads the usage of the store file system, or says why it cannot.
pub type UsageSource = Arc<dyn Fn() -> Result<DiskUsage, String> + Send + Sync>;

/// Slot of the store tasks; tasks in it run one at a time, in order.
const SLOT: &str = "store";

/// Longest a collection may run; Nix's garbage collection is safe to interrupt.
const COLLECT_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);

/// Longest the roots report may run.
const ROOTS_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// How often a waiting caller checks its task.
const POLL: Duration = Duration::from_millis(100);

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

    /// Waits for the active collection, or starts one at `priority` and waits for it.
    async fn collect(&self, priority: Priority) -> Result<(), String> {
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
        let timeout = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        let spec = TaskSpec::builder(SLOT, workload, timeout)
            .restart(RestartPolicy::Never)
            .admission(AdmissionPolicy::Queue)
            .build()
            .map_err(|error| error.to_string())?;
        let manifest = TaskManifest::new(&name, spec).map_err(|error| error.to_string())?;
        let task = self
            .supervisor
            .create_task(manifest)
            .await
            .map_err(|error| error.to_string())?;
        Ok(task.name().clone())
    }

    /// Whether the task `name` will still run: pending or running, and built by its runner.
    ///
    /// A task whose runner could not build it stays pending for good, so it is not active.
    fn active(&self, name: &TaskId) -> bool {
        self.supervisor
            .get_task(name)
            .is_some_and(|task| runs(task.status()))
    }

    /// Waits until the task `name` ends; an outcome other than success is an error.
    async fn finished(&self, name: &TaskId) -> Result<(), String> {
        let mut poll = tokio::time::interval(POLL);
        loop {
            poll.tick().await;
            let Some(task) = self.supervisor.get_task(name) else {
                return Err(format!("task {name} was removed"));
            };
            let status = task.status();
            if status.reconciliation_failed() {
                return Err(status.reconciled().message().to_owned());
            }
            let phase = status.phase();
            if phase == TaskPhase::Succeeded {
                return Ok(());
            }
            if phase.is_terminal() {
                return Err(status
                    .error()
                    .map_or_else(|| format!("task {name} ended {phase}"), ToOwned::to_owned));
            }
        }
    }
}

/// Whether a task with `status` is pending or running, and its runner built it.
pub(crate) fn runs(status: &TaskStatus) -> bool {
    status.phase().is_active() && !status.reconciliation_failed()
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
```

`crates/lmxd/src/auth.rs` turns peer credentials into a Solti identity and decides who may do what:

```rust
//! Who may do what.
//!
//! The socket is open to every local user. Each request carries the peer credentials of its
//! connection, and [`PeerIdentity`] turns them into the Solti identity `uid:<uid>`. Reading is open
//! to everyone; changing the system is for privileged callers: root, and the user `lmxd` runs as,
//! who gains nothing by asking it.

use solti::{
    api::{ApiAuthorizer, ApiError, ApiIdentity, AuthorizationRequest, TaskOperation},
    runner::async_trait,
};
use tonic::{Request, Status, service::Interceptor, transport::server::UdsConnectInfo};

/// Puts the caller's peer credentials into each request as its [`ApiIdentity`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct PeerIdentity;

impl Interceptor for PeerIdentity {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let credentials = request
            .extensions()
            .get::<UdsConnectInfo>()
            .and_then(|info| info.peer_cred)
            .ok_or_else(|| Status::unauthenticated("peer credentials are unavailable"))?;
        let mut identity = ApiIdentity::for_subject(format!("uid:{}", credentials.uid()))
            .with_attribute("uid", credentials.uid().to_string())
            .with_attribute("gid", credentials.gid().to_string());
        if let Some(pid) = credentials.pid() {
            identity = identity.with_attribute("pid", pid.to_string());
        }
        request.extensions_mut().insert(identity);
        Ok(request)
    }
}

/// Whether `identity` may change the system: root, or `own_uid`, the user `lmxd` runs as.
pub(crate) fn privileged(identity: Option<&ApiIdentity>, own_uid: u32) -> bool {
    identity
        .and_then(ApiIdentity::subject)
        .and_then(|subject| subject.strip_prefix("uid:"))
        .and_then(|uid| uid.parse::<u32>().ok())
        .is_some_and(|uid| uid == 0 || uid == own_uid)
}

/// Access to the Solti Task API: everyone reads, privileged callers cancel and delete, and nobody
/// creates or applies, because `lmxd` alone creates its tasks.
#[derive(Debug)]
pub(crate) struct TaskAccess {
    /// User `lmxd` runs as.
    pub(crate) own_uid: u32,
}

#[async_trait]
impl ApiAuthorizer for TaskAccess {
    async fn authorize(&self, request: AuthorizationRequest<'_>) -> Result<(), ApiError> {
        match request.operation() {
            TaskOperation::Get
            | TaskOperation::List
            | TaskOperation::Watch
            | TaskOperation::ListRuns
            | TaskOperation::StreamLogs => Ok(()),
            TaskOperation::Cancel | TaskOperation::Delete
                if privileged(request.identity(), self.own_uid) =>
            {
                Ok(())
            }
            TaskOperation::Cancel | TaskOperation::Delete => Err(ApiError::Forbidden(
                "only root may cancel or delete tasks of lmxd".into(),
            )),
            _ => Err(ApiError::Forbidden(
                "tasks of lmxd are created by lmxd only".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use solti::{api::TaskTarget, model::TaskId};

    use super::*;

    #[test]
    fn root_and_the_daemons_own_user_are_privileged() {
        let identity = |uid: u32| ApiIdentity::for_subject(format!("uid:{uid}"));
        assert!(privileged(Some(&identity(0)), 1000));
        assert!(privileged(Some(&identity(1000)), 1000));
        assert!(!privileged(Some(&identity(501)), 1000));
        assert!(!privileged(
            Some(&ApiIdentity::for_subject("token:ci")),
            1000
        ));
        assert!(!privileged(None, 1000));
    }

    #[tokio::test]
    async fn everyone_reads_root_cancels_and_nobody_creates_tasks() {
        let access = TaskAccess { own_uid: 1000 };
        let root = ApiIdentity::for_subject("uid:0");
        let user = ApiIdentity::for_subject("uid:501");
        let name = TaskId::new("store-collect-1").expect("valid name");
        let allowed = async |identity: &ApiIdentity, operation| {
            let request =
                AuthorizationRequest::new(Some(identity), operation, TaskTarget::Task(&name));
            access.authorize(request).await.is_ok()
        };

        for operation in [
            TaskOperation::Get,
            TaskOperation::List,
            TaskOperation::Watch,
            TaskOperation::ListRuns,
            TaskOperation::StreamLogs,
        ] {
            assert!(allowed(&user, operation).await, "{operation:?}");
        }
        for operation in [TaskOperation::Cancel, TaskOperation::Delete] {
            assert!(allowed(&root, operation).await, "{operation:?}");
            assert!(!allowed(&user, operation).await, "{operation:?}");
        }
        for operation in [TaskOperation::Create, TaskOperation::Apply] {
            assert!(!allowed(&root, operation).await, "{operation:?}");
        }
    }
}
```

`crates/lmxd/src/journal.rs` logs task output:

```rust
//! Task output in the journal.

use solti::{
    core::{TaskOutputEvent, TaskOutputSink},
    model::{OutputEvent, StreamKind},
};

/// Logs every output line of every task, so `journalctl -u lmx` shows what the tasks printed, as
/// `journalctl -u limanix-store-guard` showed the platform guard's output.
#[derive(Debug)]
pub(crate) struct Journal;

impl TaskOutputSink for Journal {
    fn on_event(&self, event: &TaskOutputEvent) {
        let task = event.task();
        match event.event() {
            OutputEvent::Chunk(chunk) => {
                let line = String::from_utf8_lossy(&chunk.line);
                match chunk.stream {
                    StreamKind::Stdout => {
                        tracing::info!(target: "lmxd::task", lmx_task = %task, "{line}")
                    }
                    StreamKind::Stderr => {
                        tracing::warn!(target: "lmxd::task", lmx_task = %task, "{line}")
                    }
                }
            }
            OutputEvent::RunFinished { exit_code, .. } => {
                tracing::info!(target: "lmxd::task", lmx_task = %task, ?exit_code, "task run finished");
            }
            _ => {}
        }
    }
}
```

`crates/lmxd/src/owner.rs` serves `lmx.v1.Owner`:

```rust
//! The `lmx.v1.Owner` service.

use std::{sync::Arc, time::UNIX_EPOCH};

use lmx_ipc::proto::{
    self, ReserveRequest, ReserveResponse, StatusRequest, StatusResponse, reserve_response::Outcome,
};
use lmx_model::{ErrorBody, ErrorCode, Operation, Owner};
use serde_json::Map;
use solti::{
    api::ApiIdentity,
    core::SupervisorApi,
    model::{TaskQuery, TaskWorkload},
};
use tonic::{Request, Response, Status};

use crate::{
    auth,
    store::{self, Store},
    tasks::API_VERSION,
};

/// Handler of `lmx.v1.Owner`.
pub(crate) struct OwnerService {
    /// Store operations.
    pub(crate) store: Arc<Store>,
    /// Supervisor whose tasks are the operations.
    pub(crate) supervisor: Arc<SupervisorApi>,
    /// User `lmxd` runs as.
    pub(crate) own_uid: u32,
}

#[tonic::async_trait]
impl proto::owner_server::Owner for OwnerService {
    async fn status(
        &self,
        _request: Request<StatusRequest>,
    ) -> Result<Response<StatusResponse>, Status> {
        let owner = Owner {
            version: env!("CARGO_PKG_VERSION").into(),
            conditions: self.store.conditions(),
            operations: operations(&self.supervisor),
        };
        Ok(Response::new(owner.into()))
    }

    async fn reserve(
        &self,
        request: Request<ReserveRequest>,
    ) -> Result<Response<ReserveResponse>, Status> {
        let caller = request.extensions().get::<ApiIdentity>();
        let outcome = if auth::privileged(caller, self.own_uid) {
            match self.store.reserve().await {
                Ok(reserve) => Outcome::Result(reserve.into()),
                Err(error) => Outcome::Failure(error.into()),
            }
        } else {
            Outcome::Failure(
                ErrorBody {
                    code: ErrorCode::PermissionDenied,
                    message: "Only root may reserve room in the store.".into(),
                    details: Map::new(),
                }
                .into(),
            )
        };
        Ok(Response::new(ReserveResponse {
            outcome: Some(outcome),
        }))
    }
}

/// Pending and running tasks of `lmxd`, without those their runner could not build.
fn operations(supervisor: &SupervisorApi) -> Vec<Operation> {
    let Ok(page) = supervisor.query_tasks(&TaskQuery::new().with_active()) else {
        return Vec::new();
    };
    page.items
        .iter()
        .filter_map(|task| {
            let TaskWorkload::Extension(workload) = task.spec().workload() else {
                return None;
            };
            (workload.api_version() == API_VERSION && store::runs(task.status())).then(|| {
                Operation {
                    task: task.name().to_string(),
                    kind: workload.kind().to_owned(),
                    phase: task.status().phase().to_string(),
                    created_at: task
                        .metadata()
                        .creation_timestamp()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |since| {
                            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
                        }),
                }
            })
        })
        .collect()
}
```

`crates/lmxd/src/daemon.rs` starts the daemon and serves the socket. Stopping follows S1: open streams end only when the
tasks stop, so the drain and the supervisor stop together:

```rust
//! Starting `lmxd` and serving its socket.

use std::{future::Future, path::Path, sync::Arc, time::Duration};

use lmx_ipc::proto::owner_server::OwnerServer;
use lmx_model::Config;
use solti::{
    api::{GrpcApi, SupervisorApiAdapter},
    core::{CoreError, SupervisorApi},
    exec::subprocess::SubprocessRunner,
    runner::RunnerRouter,
};
use tokio::{net::UnixListener, sync::Notify, task::JoinHandle};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::{service::interceptor::InterceptedService, transport::Server};

use crate::{
    auth::{PeerIdentity, TaskAccess},
    journal::Journal,
    owner::OwnerService,
    store::{self, GuardSchedule, Store, UsageSource},
    tasks::{self, RegisterError},
};

/// Longest wait for open connections, such as a client following a stream, when stopping.
const DRAIN: Duration = Duration::from_secs(5);

/// Longest wait for tasks to stop.
const STOP_TASKS: Duration = Duration::from_secs(10);

/// Longest wait for task processes to be cleaned up after the tasks stopped.
const STOP_PROCESSES: Duration = Duration::from_secs(5);

/// What `lmxd` needs besides its socket.
pub struct Options {
    /// Platform configuration: disk thresholds and tool paths.
    pub config: Config,
    /// Reads the usage of the store file system.
    pub usage: UsageSource,
    /// When the store guard checks the disk; `None` turns the guard off.
    pub guard: Option<GuardSchedule>,
}

impl std::fmt::Debug for Options {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Options")
            .field("config", &self.config)
            .field("guard", &self.guard)
            .finish_non_exhaustive()
    }
}

/// Failure to start or to serve.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The configuration cannot work.
    #[error("invalid configuration: {0}")]
    Config(String),
    /// The runners could not be registered.
    #[error(transparent)]
    Register(#[from] RegisterError),
    /// The supervisor could not start or stop.
    #[error("supervisor: {0}")]
    Supervisor(#[from] CoreError),
    /// The socket could not be served.
    #[error("serving the socket failed: {0}")]
    Serve(#[from] tonic::transport::Error),
    /// The server stopped by itself, such as after a panic.
    #[error("the server stopped unexpectedly: {0}")]
    Stopped(String),
}

/// A started daemon: tasks run, and the socket waits to be served.
pub struct Daemon {
    /// Supervisor of all tasks.
    supervisor: Arc<SupervisorApi>,
    /// Private runner of task processes; stopped after the supervisor.
    subprocess: Arc<SubprocessRunner>,
    /// Store operations.
    store: Arc<Store>,
    /// The store guard, when it runs.
    guard: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Daemon {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Daemon")
            .field("guard", &self.guard.is_some())
            .finish_non_exhaustive()
    }
}

impl Daemon {
    /// Starts the supervisor, the runners and, when scheduled, the store guard.
    pub async fn start(options: Options) -> Result<Self, Error> {
        check(&options.config)?;
        let mut router = RunnerRouter::new();
        let subprocess = tasks::register(&mut router, options.config.tools.clone())?;
        let supervisor = Arc::new(
            SupervisorApi::builder(router)
                .with_output_sink(Arc::new(Journal))
                .start()
                .await?,
        );
        let store = Arc::new(Store::new(
            Arc::clone(&supervisor),
            options.config.disk,
            options.usage,
        ));
        let guard = options
            .guard
            .map(|schedule| tokio::spawn(store::guard(Arc::clone(&store), schedule)));
        Ok(Self {
            supervisor,
            subprocess,
            store,
            guard,
        })
    }

    /// Serves `listener` until `shutdown` completes, then stops.
    ///
    /// Stopping follows S1: stop accepting connections, stop the tasks while open streams drain,
    /// because a stream ends only when its task stops, then clean up task processes.
    pub async fn serve(
        self,
        listener: UnixListener,
        shutdown: impl Future<Output = ()> + Send,
    ) -> Result<(), Error> {
        let own_uid = rustix::process::geteuid().as_raw();
        let tasks = GrpcApi::new(Arc::new(SupervisorApiAdapter::new(Arc::clone(
            &self.supervisor,
        ))))
        .with_authorizer(Arc::new(TaskAccess { own_uid }))
        .server();
        let owner = OwnerServer::new(OwnerService {
            store: Arc::clone(&self.store),
            supervisor: Arc::clone(&self.supervisor),
            own_uid,
        });

        let stop_serving = Arc::new(Notify::new());
        let mut server = tokio::spawn({
            let stop_serving = Arc::clone(&stop_serving);
            Server::builder()
                .add_service(InterceptedService::new(owner, PeerIdentity))
                .add_service(InterceptedService::new(tasks, PeerIdentity))
                .serve_with_incoming_shutdown(UnixListenerStream::new(listener), async move {
                    stop_serving.notified().await;
                })
        });

        let served = tokio::select! {
            () = shutdown => None,
            served = &mut server => Some(served),
        };
        stop_serving.notify_one();
        if let Some(guard) = &self.guard {
            guard.abort();
        }
        let (drained, stopped) = tokio::join!(
            async {
                match served {
                    Some(served) => Some(served),
                    None => tokio::time::timeout(DRAIN, &mut server).await.ok(),
                }
            },
            self.supervisor.shutdown_with_timeout(STOP_TASKS),
        );
        if drained.is_none() {
            tracing::warn!("connections were still open after {DRAIN:?}; stopping anyway");
            server.abort();
        }
        if let Err(error) = self.subprocess.shutdown(STOP_PROCESSES).await {
            tracing::warn!(%error, "task processes were not cleaned up");
        }
        stopped?;
        match drained {
            Some(Ok(result)) => result.map_err(Error::Serve),
            Some(Err(stopped)) => Err(Error::Stopped(stopped.to_string())),
            None => Ok(()),
        }
    }
}

/// Rejects a configuration `lmxd` cannot work with, before anything starts.
fn check(config: &Config) -> Result<(), Error> {
    let tools = &config.tools;
    for (name, path) in [
        ("nix_store", &tools.nix_store),
        ("nice", &tools.nice),
        ("ionice", &tools.ionice),
        ("grep", &tools.grep),
    ] {
        if !Path::new(path).is_absolute() {
            return Err(Error::Config(format!(
                "tools.{name} must be an absolute path, not {path:?}"
            )));
        }
    }
    let disk = config.disk;
    if disk.minimum_percent > disk.collect_percent || disk.collect_percent > 100 {
        return Err(Error::Config(format!(
            "disk thresholds must satisfy minimum_percent <= collect_percent <= 100, not {} and {}",
            disk.minimum_percent, disk.collect_percent
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Configuration with absolute tools and the platform thresholds.
    fn config() -> Config {
        let tool = |name: &str| format!("/run/current-system/sw/bin/{name}");
        let json = serde_json::json!({
            "schema": 1,
            "vm": {"name": "dev-box", "arch": "arm64", "system": "NixOS 26.05"},
            "generation": "0123456789ab",
            "user": {"name": "dev", "home": "/home/dev", "uid": 501},
            "modules": [],
            "disk": {"collect_percent": 20, "minimum_percent": 10},
            "session": {"command": null, "providers": []},
            "tools": {
                "ip": tool("ip"),
                "systemctl": tool("systemctl"),
                "nix_store": tool("nix-store"),
                "nice": tool("nice"),
                "ionice": tool("ionice"),
                "grep": tool("grep")
            }
        });
        Config::from_json(json.to_string().as_bytes()).expect("valid configuration")
    }

    #[test]
    fn accepts_the_platform_configuration() {
        assert!(check(&config()).is_ok());
    }

    #[test]
    fn rejects_a_tool_without_an_absolute_path() {
        let mut config = config();
        config.tools.nix_store = "nix-store".into();
        let error = check(&config).expect_err("relative tool");
        assert!(error.to_string().contains("tools.nix_store"), "{error}");
    }

    #[test]
    fn rejects_a_minimum_above_the_collect_threshold() {
        let mut config = config();
        config.disk.minimum_percent = 30;
        assert!(check(&config).is_err());
    }
}
```

`crates/lmxd/src/lib.rs`:

```rust
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
```

`crates/lmxd/src/main.rs`, the binary. Started without a socket from systemd, as the transient daemon of an update will
be, it binds the socket itself with mode `0666`:

```rust
//! `lmxd`: the guest owner daemon of a LimaNix VM.
//!
//! systemd starts it as root through `lmx.socket`, which passes the listening socket, so the socket
//! exists while the daemon restarts. Started without one, as the transient daemon of an update,
//! it binds the socket itself. It reports readiness and feeds the watchdog when systemd asks for
//! them, and stops in order on SIGTERM or SIGINT.
#![forbid(unsafe_code)]

use std::{
    fs::{self, Permissions},
    io,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use clap::Parser;
use lmx_facts::disk::{self, STORE_PATH};
use lmx_ipc::SOCKET_PATH;
use lmx_model::{CONFIG_PATH, Config};
use lmxd::{Daemon, GuardSchedule, Options};
use sd_notify::NotifyState;
use solti::observe::{LoggerConfig, LoggerFormat, init_logger};
use tokio::{
    net::UnixListener,
    signal::unix::{SignalKind, signal},
    time::MissedTickBehavior,
};

/// Guest owner daemon of a LimaNix VM.
#[derive(Debug, Parser)]
#[command(name = "lmxd", version)]
struct Args {
    /// Platform configuration.
    #[arg(long, default_value = CONFIG_PATH)]
    config: PathBuf,
    /// Socket to bind when systemd passes none.
    #[arg(long, default_value = SOCKET_PATH)]
    socket: PathBuf,
}

fn main() -> ExitCode {
    let args = Args::parse();
    logging();
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("lmxd: cannot start the runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            eprintln!("lmxd: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Starts the daemon and serves until a stop signal.
async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load(&args.config)?;
    let listener = listener(&args.socket)?;
    let daemon = Daemon::start(Options {
        config,
        usage: Arc::new(|| disk::usage(Path::new(STORE_PATH)).map_err(|error| error.to_string())),
        guard: Some(GuardSchedule::after_boot(uptime())),
    })
    .await?;
    let _ = sd_notify::notify(false, &[NotifyState::Ready]);
    feed_watchdog();
    tracing::info!("lmxd {} is ready", env!("CARGO_PKG_VERSION"));
    daemon.serve(listener, stop_signal()).await?;
    tracing::info!("lmxd stopped");
    Ok(())
}

/// Logs to journald under systemd and to standard error otherwise.
fn logging() {
    let format = if std::env::var_os("JOURNAL_STREAM").is_some() {
        LoggerFormat::Journald
    } else {
        LoggerFormat::Text
    };
    if let Err(error) = init_logger(&LoggerConfig {
        format,
        ..LoggerConfig::default()
    }) {
        eprintln!("lmxd: logging is unavailable: {error}");
    }
}

/// The socket systemd passed, or a socket bound at `path` that every user may connect to.
fn listener(path: &Path) -> io::Result<UnixListener> {
    if let Some(listener) = listenfd::ListenFd::from_env().take_unix_listener(0)? {
        listener.set_nonblocking(true)?;
        return UnixListener::from_std(listener);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // A socket left by an earlier daemon is replaced; anything else at the path is not ours.
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path)?,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} exists and is not a socket", path.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(path)?;
    // Every user may connect; peer credentials decide what each caller may do.
    fs::set_permissions(path, Permissions::from_mode(0o666))?;
    Ok(listener)
}

/// How long the system has been up; unknown counts as long, so the guard checks at once.
fn uptime() -> Duration {
    fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse::<f64>().ok())
        .map_or(Duration::MAX, Duration::from_secs_f64)
}

/// Feeds the systemd watchdog at half its interval, when systemd set one.
fn feed_watchdog() {
    let mut usec = 0;
    if !sd_notify::watchdog_enabled(false, &mut usec) {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_micros(usec / 2));
        // After a stall, one ping is enough; a burst would hide how long the stall was.
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let _ = sd_notify::notify(false, &[NotifyState::Watchdog]);
        }
    });
}

/// Completes on SIGTERM or SIGINT, after telling systemd that the daemon is stopping.
async fn stop_signal() {
    let (Ok(mut terminate), Ok(mut interrupt)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        tracing::error!("stop signals cannot be handled; waiting for SIGKILL");
        return std::future::pending().await;
    };
    tokio::select! {
        _ = terminate.recv() => {}
        _ = interrupt.recv() => {}
    }
    let _ = sd_notify::notify(false, &[NotifyState::Stopping]);
    tracing::info!("stopping");
}
```

**Step 3: Run the crate's tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmxd`

Expected: PASS (13 tests): commands of the kinds, the roots filter with the real `grep`, the guard schedule, shortage
details, configuration checks and the authorization matrix.

---

## Task 4: `lmx` as the client of `lmxd`

**Files:**
- Create: `crates/lmx/src/owner.rs`, `crates/lmx/src/store.rs`
- Modify: `crates/lmx/Cargo.toml`, `crates/lmx/src/output.rs`, `crates/lmx/src/system.rs`, `crates/lmx/src/cli.rs`, `crates/lmx/src/main.rs`, `crates/lmx/src/status.rs`, `crates/lmx/tests/cli.rs`
- Test: `crates/lmx/tests/owner.rs`

`lmx store reserve` asks `lmxd` and reports its answer; `lmx status` adds the owner part. The socket is
`run/lmx/lmx.sock` below `LMX_SYSTEM_ROOT`, so the existing guest trees in the tests have no daemon and new tests can
start one.

**Step 1: Add the dependencies**

In `crates/lmx/Cargo.toml`:

```toml
lmx-facts     = { workspace = true }
lmx-model     = { workspace = true }
```

with:

```toml
lmx-facts     = { workspace = true }
lmx-ipc       = { workspace = true }
lmx-model     = { workspace = true }
```

```toml
serde_json    = { workspace = true }
unicode-width = { workspace = true }
```

with:

```toml
serde_json    = { workspace = true }
tokio         = { workspace = true, features = ["rt", "time"] }
unicode-width = { workspace = true }
```

The tests start `lmxd` in process:

```toml
[dev-dependencies]
tempfile = { workspace = true }
```

with:

```toml
[dev-dependencies]
lmxd     = { workspace = true }
tempfile = { workspace = true }
tokio    = { workspace = true, features = ["net", "rt-multi-thread", "sync"] }
```

**Step 2: Write the failing tests**

The integration scenarios of the design. Create `crates/lmx/tests/owner.rs`; a fake `nix-store` turns the scripted disk
from `before` into `after` when it collects, and fake `nice` and `ionice` log the guard's priority chain:

```rust
//! `lmx` with a running `lmxd`: the owner part of `status`, and `store reserve`.
//!
//! Each test starts `lmxd` in this process on the socket of a prepared guest tree. The store disk is
//! read from a file, and a fake `nix-store` rewrites that file when it collects, so a test decides
//! what a collection frees.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use lmx_model::{Config, DiskUsage};
use lmxd::{Daemon, GuardSchedule, Options, UsageSource};
use serde_json::{Value, json};
use tokio::{net::UnixListener, runtime::Runtime, sync::oneshot, task::JoinHandle};

/// Fake `nix-store`. It logs its arguments next to the guest tree's `bin`. A collection takes two
/// seconds, so concurrent callers overlap, and then turns the disk into `disk-after.json`. A roots
/// listing prints a root the report leaves out and one it keeps.
const NIX_STORE: &str = r#"#!/bin/sh
root=$(dirname "$(dirname "$0")")
printf '%s\n' "$*" >> "$root/calls"
case "$*" in
  "--gc --quiet") sleep 2; cp "$root/disk-after.json" "$root/disk.json" ;;
  "--gc --print-roots")
    echo '/proc/1/environ -> /nix/store/aaaa-glibc'
    echo '/home/dev/project/.direnv/flake-inputs/cccc-source -> /nix/store/cccc-source' ;;
esac
"#;

/// Fake `nice` or `ionice`: logs its name and arguments, drops its two options and runs the rest.
fn priority_tool(name: &str) -> String {
    format!(
        r#"#!/bin/sh
root=$(dirname "$(dirname "$0")")
printf '{name} %s\n' "$*" >> "$root/calls"
shift 2
exec "$@"
"#
    )
}

/// A 1000 MiB store disk with `free_percent` of its bytes free and plenty of inodes.
fn usage(free_percent: u64) -> DiskUsage {
    DiskUsage {
        bytes: 1000 << 20,
        free_bytes: (free_percent * 10) << 20,
        available_bytes: (free_percent * 10) << 20,
        inodes: 1000,
        free_inodes: 500,
    }
}

/// A guest tree with `lmxd` serving its socket.
struct Guest {
    /// Root replacing `/`; removed when the test ends.
    root: tempfile::TempDir,
    /// Runtime of the daemon.
    runtime: Runtime,
    /// Stops the daemon.
    stop: Option<oneshot::Sender<()>>,
    /// The serving daemon.
    served: Option<JoinHandle<Result<(), lmxd::Error>>>,
}

impl Guest {
    /// A guest whose store disk is `before` until a collection makes it `after`.
    fn new(before: DiskUsage, after: DiskUsage) -> Self {
        Self::start(before, after, None)
    }

    /// Like [`Guest::new`], with a store guard that checks the disk at once.
    fn with_guard(before: DiskUsage, after: DiskUsage) -> Self {
        let guard = GuardSchedule {
            first: Duration::ZERO,
            every: Duration::from_secs(3600),
        };
        Self::start(before, after, Some(guard))
    }

    /// Prepares the tree and starts `lmxd` with `guard`.
    fn start(before: DiskUsage, after: DiskUsage, guard: Option<GuardSchedule>) -> Self {
        let root = tempfile::tempdir().expect("temporary guest root");
        let path = |relative: &str| root.path().join(relative);
        fs::write(path("disk.json"), json!(before).to_string()).expect("write the disk");
        fs::write(path("disk-after.json"), json!(after).to_string()).expect("write the disk");
        fs::create_dir_all(path("bin")).expect("create bin");
        fs::create_dir_all(path("nix/store")).expect("create the store");
        fs::create_dir_all(path("run/lmx")).expect("create the socket directory");
        fs::create_dir_all(path("etc/lmx")).expect("create the configuration directory");
        for (tool, script) in [
            ("nix-store", NIX_STORE.to_owned()),
            ("nice", priority_tool("nice")),
            ("ionice", priority_tool("ionice")),
        ] {
            let tool = path(&format!("bin/{tool}"));
            fs::write(&tool, script).expect("write a tool");
            fs::set_permissions(&tool, fs::Permissions::from_mode(0o755))
                .expect("make a tool executable");
        }

        let config = json!({
            "schema": 1,
            "vm": {"name": "dev-box", "arch": "arm64", "system": "NixOS 26.05"},
            "generation": "0123456789ab",
            "user": {"name": "dev", "home": "/home/dev", "uid": 501},
            "modules": [],
            "disk": {"collect_percent": 20, "minimum_percent": 10},
            "session": {"command": null, "providers": []},
            "tools": {
                "ip": path("bin/ip"),
                "systemctl": path("bin/systemctl"),
                "nix_store": path("bin/nix-store"),
                "nice": path("bin/nice"),
                "ionice": path("bin/ionice"),
                "grep": "/usr/bin/grep"
            }
        });
        fs::write(path("etc/lmx/config.json"), config.to_string())
            .expect("write the configuration");
        let config = Config::from_json(config.to_string().as_bytes()).expect("valid configuration");

        let disk = path("disk.json");
        let usage: UsageSource = Arc::new(move || {
            let data = fs::read(&disk).map_err(|error| error.to_string())?;
            serde_json::from_slice(&data).map_err(|error| error.to_string())
        });
        let runtime = Runtime::new().expect("runtime");
        let daemon = runtime
            .block_on(Daemon::start(Options {
                config,
                usage,
                guard,
            }))
            .expect("lmxd starts");
        let listener = {
            let _runtime = runtime.enter();
            UnixListener::bind(path("run/lmx/lmx.sock")).expect("bind the socket")
        };
        let (stop, stopped) = oneshot::channel::<()>();
        let served = runtime.spawn(daemon.serve(listener, async {
            let _ = stopped.await;
        }));
        Self {
            root,
            runtime,
            stop: Some(stop),
            served: Some(served),
        }
    }

    /// Prepares `lmx` with the tree as its system root.
    fn lmx(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lmx"));
        command
            .args(args)
            .env("LMX_SYSTEM_ROOT", self.root.path())
            .env("LMX_CONFIG", self.path("etc/lmx/config.json"))
            .env("PATH", self.path("empty"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    /// Path of `relative` below the root.
    fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    /// Arguments of every `nix-store` call so far.
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.path("calls"))
            .unwrap_or_default()
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }
}

impl Drop for Guest {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(served) = self.served.take() {
            let _ = self.runtime.block_on(served);
        }
    }
}

/// Parses standard output as one JSON answer, whatever the exit status.
fn answer(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "standard output is one JSON answer ({error}): {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn concurrent_reserves_share_one_collection() {
    let guest = Guest::new(usage(15), usage(30));
    let first = guest
        .lmx(&["store", "reserve", "--json"])
        .spawn()
        .expect("run lmx");
    let second = guest
        .lmx(&["store", "reserve", "--json"])
        .spawn()
        .expect("run lmx");

    for child in [first, second] {
        let output = child.wait_with_output().expect("wait for lmx");
        assert!(output.status.success(), "{output:?}");
        let answer = answer(&output);
        assert_eq!(answer["ok"], true);
        assert_eq!(answer["data"]["collected"], true);
        assert_eq!(answer["data"]["freed_bytes"], 150 << 20);
        assert_eq!(answer["data"]["after"]["free_bytes"], 300 << 20);
    }
    assert_eq!(guest.calls(), ["--gc --quiet"]);
}

#[test]
fn a_disk_that_stays_low_is_reported_with_its_roots() {
    let guest = Guest::new(usage(5), usage(8));
    let output = guest
        .lmx(&["store", "reserve", "--json"])
        .output()
        .expect("run lmx");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let answer = answer(&output);
    assert_eq!(answer["ok"], false);
    assert_eq!(answer["error"]["code"], "disk.low");
    assert_eq!(answer["error"]["details"]["freed_bytes"], 30 << 20);
    // The roots report runs for the journal after the answer.
    let deadline = Instant::now() + Duration::from_secs(10);
    while guest.calls().len() < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(guest.calls(), ["--gc --quiet", "--gc --print-roots"]);
}

#[test]
fn the_guard_collects_at_idle_priority() {
    let guest = Guest::with_guard(usage(15), usage(30));
    let deadline = Instant::now() + Duration::from_secs(10);
    while guest.calls().len() < 3 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }

    let nix_store = guest.path("bin/nix-store").display().to_string();
    let ionice = guest.path("bin/ionice").display().to_string();
    assert_eq!(
        guest.calls(),
        [
            format!("nice -n 19 {ionice} -c 3 {nix_store} --gc --quiet"),
            format!("ionice -c 3 {nix_store} --gc --quiet"),
            "--gc --quiet".to_owned(),
        ]
    );
}

#[test]
fn status_includes_the_owner_when_lmxd_runs() {
    let guest = Guest::new(usage(5), usage(5));
    let output = guest.lmx(&["status", "--json"]).output().expect("run lmx");
    assert!(output.status.success(), "{output:?}");

    let owner = &answer(&output)["data"]["owner"];
    assert_eq!(owner["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(owner["conditions"][0]["type"], "DiskLow");
    assert_eq!(owner["operations"], json!([]));
    assert!(guest.calls().is_empty(), "status collects nothing");
}
```

Without a daemon, `lmx status` now reports the owner as an unreadable fact. In `crates/lmx/tests/cli.rs`:

```rust
    assert_eq!(data["failed_units"], json!(["limanix-store-guard.service"]));
    assert!(
        data.get("problems").is_none(),
        "every fact was read: {data}"
    );
```

with:

```rust
    assert_eq!(data["failed_units"], json!(["limanix-store-guard.service"]));
    // No lmxd runs in this guest: the owner is the only fact that cannot be read.
    assert_eq!(data["owner"], Value::Null);
    assert_eq!(data["problems"].as_array().map(Vec::len), Some(1), "{data}");
    assert_eq!(data["problems"][0]["fact"], "owner");
    assert!(
        data["problems"][0]["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("lmxd is not reachable at ")),
        "{data}"
    );
```

```rust
    assert_eq!(
        facts,
        ["config", "generations", "interfaces", "failed_units"]
    );
```

with:

```rust
    assert_eq!(
        facts,
        [
            "config",
            "generations",
            "interfaces",
            "failed_units",
            "owner"
        ]
    );
```

```rust
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0]["fact"], "disk");
```

with:

```rust
    let facts: Vec<&Value> = problems.iter().map(|problem| &problem["fact"]).collect();
    assert_eq!(facts, ["disk", "owner"], "{problems:?}");
```

Without a daemon, `lmx store reserve` must answer `owner.unavailable` with exit status 3, the only producer of that
status:

```rust
#[test]
fn store_reserve_without_lmxd_reports_an_unavailable_owner() {
    let guest = Guest::new();
    let output = guest.lmx(&["store", "reserve", "--json"], &guest.config());
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let answer: Value = serde_json::from_slice(&output.stdout).expect("one JSON answer");
    assert_eq!(answer["ok"], false);
    assert_eq!(answer["error"]["code"], "owner.unavailable");
}
```

**Step 3: Run the tests to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx --no-fail-fast --test owner --test cli`

Expected: FAIL: `lmx store` is not a command yet, and `status` reports no owner problem.

**Step 4: Ask `lmxd`**

In `crates/lmx/src/output.rs`, the exit status of the contract:

```rust
/// Exit status of a usage error.
const USAGE: u8 = 2;
```

with:

```rust
/// Exit status of a usage error.
const USAGE: u8 = 2;

/// Exit status when the guest owner daemon `lmxd` is unavailable.
pub(crate) const UNAVAILABLE: u8 = 3;
```

In `crates/lmx/src/system.rs`, the socket below the system root:

```rust
use lmx_model::{CONFIG_PATH, Config};
```

with:

```rust
use lmx_ipc::SOCKET_PATH;
use lmx_model::{CONFIG_PATH, Config};
```

and:

```rust
    /// Locations of the generation markers.
```

add before it:

```rust
    /// Socket of the guest owner daemon `lmxd`.
    pub(crate) fn owner_socket(&self) -> PathBuf {
        self.root.join(SOCKET_PATH.trim_start_matches('/'))
    }
```

Create `crates/lmx/src/owner.rs`. A daemon that cannot be reached is reported, never replaced; `status` waits at most
two seconds, `reserve` as long as the collection takes:

```rust
//! Calls to the guest owner daemon `lmxd`.
//!
//! Owner operations run only in `lmxd`; `lmx` asks for them over the daemon's socket and never runs
//! them itself. A daemon that cannot be reached is reported, never replaced.

use std::{fmt, future::Future, path::Path, time::Duration};

use lmx_ipc::{
    proto::{ReserveRequest, StatusRequest, owner_client::OwnerClient},
    tonic::{Code, Status, transport::Channel},
};
use lmx_model::{ErrorBody, Owner, Reserve};

/// How long `lmx status` waits for `lmxd` to connect and answer.
const STATUS_TIMEOUT: Duration = Duration::from_secs(2);

/// How long `lmx store reserve` waits for `lmxd` to answer at all before it asks for the reserve.
///
/// systemd accepts connections on the socket before the daemon runs, so a daemon that never
/// starts would otherwise keep the host waiting; this is long enough for a socket-activated start.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Why a call to `lmxd` gave no answer of the host contract.
#[derive(Debug)]
pub(crate) enum CallError {
    /// `lmxd` could not be reached, did not answer in time, or reported itself unavailable.
    Unavailable(String),
    /// `lmxd` answered outside its protocol, or the call could not be made.
    Failed(String),
}

impl CallError {
    /// Error of a call that ended with gRPC `status`.
    fn from_status(status: &Status) -> Self {
        let message = format!("lmxd did not answer: {}", status.message());
        if status.code() == Code::Unavailable {
            Self::Unavailable(message)
        } else {
            Self::Failed(message)
        }
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(message) | Self::Failed(message) => formatter.write_str(message),
        }
    }
}

/// Asks `lmxd` on `socket` for its state, waiting at most [`STATUS_TIMEOUT`].
pub(crate) fn status(socket: &Path) -> Result<Owner, CallError> {
    block_on(async {
        let call = async {
            let mut client = connect(socket).await?;
            let response = client
                .status(StatusRequest {})
                .await
                .map_err(|status| CallError::from_status(&status))?;
            Ok(Owner::from(response.into_inner()))
        };
        tokio::time::timeout(STATUS_TIMEOUT, call)
            .await
            .unwrap_or_else(|_| Err(silent(STATUS_TIMEOUT)))
    })
}

/// Asks `lmxd` on `socket` to make room in the store, waiting as long as the collection takes.
///
/// `lmxd` must first answer a status call within [`PROBE_TIMEOUT`], so a daemon that never starts is
/// reported as unavailable instead of waited for.
pub(crate) fn reserve(socket: &Path) -> Result<Result<Reserve, ErrorBody>, CallError> {
    block_on(async {
        let mut client = connect(socket).await?;
        tokio::time::timeout(PROBE_TIMEOUT, client.status(StatusRequest {}))
            .await
            .map_err(|_| silent(PROBE_TIMEOUT))?
            .map_err(|status| CallError::from_status(&status))?;
        let response = client
            .reserve(ReserveRequest {})
            .await
            .map_err(|status| CallError::from_status(&status))?;
        lmx_ipc::reserve_outcome(response.into_inner())
            .map_err(|error| CallError::Failed(error.to_string()))
    })
}

/// Error of a daemon that did not answer within `timeout`.
fn silent(timeout: Duration) -> CallError {
    CallError::Unavailable(format!(
        "lmxd did not answer within {} seconds",
        timeout.as_secs()
    ))
}

/// Connects to `lmxd` on `socket`.
async fn connect(socket: &Path) -> Result<OwnerClient<Channel>, CallError> {
    lmx_ipc::connect(socket).await.map_err(|error| {
        CallError::Unavailable(format!(
            "lmxd is not reachable at {}: {error}",
            socket.display()
        ))
    })
}

/// Runs `call` on a runtime of its own; `lmx` makes one call per command.
fn block_on<T>(call: impl Future<Output = Result<T, CallError>>) -> Result<T, CallError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| CallError::Failed(format!("cannot start the async runtime: {error}")))?
        .block_on(call)
}
```

Create `crates/lmx/src/store.rs`:

```rust
//! `lmx store reserve`: room in the Nix store before the host stops the VM.
//!
//! The collection runs in `lmxd`, which keeps going if this command is interrupted. The answer
//! carries the store disk usage before and after, so the host can warn about a disk that stays low.

use std::{io, process::ExitCode};

use lmx_model::{Envelope, ErrorBody, ErrorCode, Reserve, Shortage};
use serde_json::{Map, Value};

use crate::{
    cli::OutputArgs,
    format, output,
    owner::{self, CallError},
    system::System,
};

/// Runs `lmx store reserve`.
pub(crate) fn reserve(system: &System, args: &OutputArgs) -> io::Result<ExitCode> {
    match owner::reserve(&system.owner_socket()) {
        Ok(Ok(reserve)) => {
            if args.json {
                output::write_json(&Envelope::success(&reserve))?;
            } else {
                output::write_text(&render(&reserve))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Ok(Err(error)) => fail(args, error, output::FAILURE),
        Err(CallError::Unavailable(message)) => fail(
            args,
            ErrorBody {
                code: ErrorCode::OwnerUnavailable,
                message,
                details: Map::new(),
            },
            output::UNAVAILABLE,
        ),
        Err(error @ CallError::Failed(_)) => {
            eprintln!("lmx: {error}");
            Ok(ExitCode::from(output::FAILURE))
        }
    }
}

/// Reports `error` and exits with `status`.
fn fail(args: &OutputArgs, error: ErrorBody, status: u8) -> io::Result<ExitCode> {
    if args.json {
        output::write_json(&Envelope::<()>::failure(error))?;
    } else {
        eprintln!("lmx: {}", error.message);
        if let Ok(shortage) = serde_json::from_value::<Shortage>(Value::Object(error.details)) {
            eprintln!("Guest disk: {}.", format::disk(&shortage.after));
        }
    }
    Ok(ExitCode::from(status))
}

/// Renders a reserve for people.
fn render(reserve: &Reserve) -> String {
    let done = if reserve.collected {
        format!(
            "Collected unreferenced store paths and freed {}.",
            format::gibibytes(reserve.freed_bytes)
        )
    } else {
        "Nothing was collected.".to_owned()
    };
    format!("{done}\nGuest disk: {}.\n", format::disk(&reserve.after))
}

#[cfg(test)]
mod tests {
    use lmx_model::DiskUsage;

    use super::*;

    /// 16 GiB disk with `free` GiB available.
    fn usage(free: u64) -> DiskUsage {
        DiskUsage {
            bytes: 16 << 30,
            free_bytes: free << 30,
            available_bytes: free << 30,
            inodes: 0,
            free_inodes: 0,
        }
    }

    #[test]
    fn says_how_much_a_collection_freed() {
        let reserve = Reserve {
            before: usage(2),
            after: usage(6),
            freed_bytes: 4 << 30,
            collected: true,
        };
        assert_eq!(
            render(&reserve),
            "Collected unreferenced store paths and freed 4.0 GiB.\n\
             Guest disk: 6.0 GiB of 16 GiB free.\n"
        );
    }

    #[test]
    fn says_when_nothing_was_collected() {
        let reserve = Reserve {
            before: usage(8),
            after: usage(8),
            freed_bytes: 0,
            collected: false,
        };
        assert_eq!(
            render(&reserve),
            "Nothing was collected.\nGuest disk: 8.0 GiB of 16 GiB free.\n"
        );
    }
}
```

In `crates/lmx/src/cli.rs`, the command:

```rust
    /// Show the lmx version and the host contract it speaks.
    Version(OutputArgs),
```

with:

```rust
    /// Show the lmx version and the host contract it speaks.
    Version(OutputArgs),
    /// Keep room in the Nix store; the work runs in lmxd.
    #[command(subcommand)]
    Store(StoreCommand),
```

and its subcommands:

```rust
/// Clipboard operations.
```

add before it:

```rust
/// Store operations.
#[derive(Debug, Subcommand)]
pub(crate) enum StoreCommand {
    /// Collect unreferenced store paths when free space is low; the host runs it before it stops
    /// the VM.
    Reserve(OutputArgs),
}
```

In `crates/lmx/src/main.rs`, the crate documentation gains the command, the owner kind and the socket under
`LMX_SYSTEM_ROOT`. Replace it through the `#![forbid(unsafe_code)]` line:

```rust
//! # lmx
//!
//! Command of the LimaNix guest owner. People run it inside the VM; the LimaNix host runs it over
//! management SSH with `--json` and reads the [host contract](https://github.com/limanix/lmx/blob/main/docs/contract.md).
//!
//! | Command               | Kind   | Answers or does                                             |
//! |-----------------------|--------|-------------------------------------------------------------|
//! | `lmx help`            | facts  | the workspace and the commands inside the VM and on the Mac |
//! | `lmx info`            | facts  | kernel, guest disk, shared folders and failed units         |
//! | `lmx welcome`         | caller | the summary an interactive shell shows when it starts       |
//! | `lmx status`          | facts  | generations, disk, interfaces, failed units, and `lmxd`     |
//! | `lmx version`         | facts  | the binary version and the host contract it speaks          |
//! | `lmx store reserve`   | owner  | room in the store before the host stops the VM, in `lmxd`   |
//! | `lmx clipboard copy`  | caller | copies standard input to the Mac clipboard                  |
//! | `lmx clipboard paste` | caller | prints the Mac clipboard, if the terminal allows reads      |
//! | `lmx session NAME`    | caller | opens a named session with the selected provider            |
//!
//! Facts are read in the caller's process with the caller's privileges and need no daemon. Owner
//! operations run only in the guest owner daemon `lmxd`; without it they fail with exit status 3.
//! Caller commands act on the caller's terminal and environment, so only the caller can run them.
//!
//! ## Other names
//!
//! Started under the name of a shell command it replaces, the binary runs that command. The
//! arguments, messages and exit statuses stay those of the replaced command.
//!
//! | Name                   | Runs                  |
//! |------------------------|-----------------------|
//! | `pbcopy`               | `lmx clipboard copy`  |
//! | `pbpaste`              | `lmx clipboard paste` |
//! | `limanix-session NAME` | `lmx session NAME`    |
//!
//! ## Test hooks
//!
//! Environment variables let tests point the binary at prepared files. `sudo` drops all of them by
//! default, so the host never sets them by accident.
//!
//! | Variable            | Replaces                                                                  |
//! |---------------------|---------------------------------------------------------------------------|
//! | `LMX_CONFIG`        | [`lmx_model::CONFIG_PATH`]                                                |
//! | `LMX_SYSTEM_ROOT`   | `/` for generation markers, the store path, `/proc` and the `lmxd` socket |
//! | `LMX_TTY_IN`        | `/dev/tty` for reading the terminal's clipboard reply                     |
//! | `LMX_TTY_OUT`       | `/dev/tty` for writing clipboard sequences                                |
//! | `LMX_PASTE_TIMEOUT` | the 10 seconds `pbpaste` waits for a reply, in seconds                    |
```

The modules:

```rust
mod output;
mod palette;
mod process;
mod session;
mod status;
mod system;
```

with:

```rust
mod output;
mod owner;
mod palette;
mod process;
mod session;
mod status;
mod store;
mod system;
```

the import:

```rust
    cli::{Cli, ClipboardCommand, Command},
```

with:

```rust
    cli::{Cli, ClipboardCommand, Command, StoreCommand},
```

and the dispatch:

```rust
        Some(Command::Version(args)) => version::run(&args),
```

with:

```rust
        Some(Command::Version(args)) => version::run(&args),
        Some(Command::Store(StoreCommand::Reserve(args))) => {
            store::reserve(&System::from_environment(), &args)
        }
```

In `crates/lmx/src/status.rs`, the module documentation:

```rust
//! [`Problem`], so the host and people always get the rest. The owner part comes from `lmxd` when it
//! answers within two seconds.
```

the import:

```rust
use crate::{cli::OutputArgs, format, layout, output, system::System};
```

with:

```rust
use crate::{cli::OutputArgs, format, layout, output, owner, system::System};
```

the owner in `collect`, which replaces the placeholder of Task 1:

```rust
        owner: None,
        problems,
```

with:

```rust
        owner: owner::status(&system.owner_socket())
            .map_err(|error| {
                problems.push(Problem {
                    fact: "owner".into(),
                    message: error.to_string(),
                });
            })
            .ok(),
        problems,
```

and in `render`, an owner row and one row per condition, before the problems:

```rust
    for problem in &status.problems {
```

with:

```rust
    rows.push((
        "Owner",
        status.owner.as_ref().map_or_else(unknown, |owner| {
            let doing: Vec<String> = owner
                .operations
                .iter()
                .map(|operation| format!("{} {}", operation.kind, operation.phase))
                .collect();
            let doing = if doing.is_empty() {
                "idle".to_owned()
            } else {
                doing.join(", ")
            };
            format!("lmxd {}, {doing}", owner.version)
        }),
    ));
    for condition in status.owner.iter().flat_map(|owner| &owner.conditions) {
        rows.push(("Condition", condition.message.clone()));
    }
    for problem in &status.problems {
```

Its unit tests follow the new rows:

```rust
    use lmx_model::{DiskUsage, Generations, Interface};
```

with:

```rust
    use lmx_model::{Condition, DiskUsage, Generations, Interface, Operation, Owner};
```

```rust
            failed_units: Some(vec![]),
            owner: None,
            problems: vec![],
        };
        assert_eq!(
            render(&status),
            "Generation    desired 0123456789ab, built 0123456789ab, booted unknown\n\
             Disk          8.0 GiB of 16 GiB free, 495616 of 1048576 inodes free\n\
             Network       enp0s1 192.0.2.10\n\
             Failed units  none\n"
        );
```

with:

```rust
            failed_units: Some(vec![]),
            owner: Some(Owner {
                version: "0.1.0".into(),
                conditions: vec![Condition {
                    kind: "DiskLow".into(),
                    message: "Less than 10% of the guest disk is free.".into(),
                }],
                operations: vec![Operation {
                    task: "store-collect-1".into(),
                    kind: "StoreCollect".into(),
                    phase: "running".into(),
                    created_at: 0,
                }],
            }),
            problems: vec![],
        };
        assert_eq!(
            render(&status),
            "Generation    desired 0123456789ab, built 0123456789ab, booted unknown\n\
             Disk          8.0 GiB of 16 GiB free, 495616 of 1048576 inodes free\n\
             Network       enp0s1 192.0.2.10\n\
             Failed units  none\n\
             Owner         lmxd 0.1.0, StoreCollect running\n\
             Condition     Less than 10% of the guest disk is free.\n"
        );
```

```rust
        let problem: Vec<&str> = text.lines().skip(4).collect();
```

with:

```rust
        let problem: Vec<&str> = text.lines().skip(5).collect();
```

**Step 5: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx`

Expected: PASS (69 tests), including the four owner scenarios and the unavailable owner.

---

## Task 5: Reference units and the release archive

**Files:**
- Create: `packaging/systemd/lmx.socket`, `packaging/systemd/lmx.service`
- Modify: `Taskfile.yml`

The platform base (M1c) renders its own units with store paths; these references fix the settings the daemon relies on.

`packaging/systemd/lmx.socket`:

```ini
# Reference unit for the LimaNix platform base, which renders its own copy.
# The socket outlives lmx.service, so calls wait in the backlog while the daemon restarts.
[Unit]
Description=Socket of the LimaNix guest owner

[Socket]
ListenStream=/run/lmx/lmx.sock
# Every user may connect; lmxd decides what each caller may do from its peer credentials.
SocketMode=0666

[Install]
WantedBy=sockets.target
```

`packaging/systemd/lmx.service`:

```ini
# Reference unit for the LimaNix platform base, which renders its own copy with store paths.
[Unit]
Description=LimaNix guest owner
Requires=lmx.socket
After=lmx.socket

[Service]
Type=notify
ExecStart=/run/current-system/sw/bin/lmxd
# lmxd pings at half this interval; a frozen daemon is aborted and restarted.
WatchdogSec=30s
Restart=on-failure
# Task processes live in this unit's control group, so stopping the unit never leaves them behind.
KillMode=control-group
# A watchdog abort would otherwise store a core of every process in the unit.
LimitCORE=0

[Install]
# Started at boot, not only by its socket: the store guard must run.
WantedBy=multi-user.target
```

In `Taskfile.yml`, `release/build` builds and packs `lmxd` next to `lmx`:

```yaml
                cargo build --release --locked --target "$target" -p lmx
```

with:

```yaml
                cargo build --release --locked --target "$target" -p lmx -p lmxd
```

```yaml
                install -m 0755 "$CARGO_TARGET_DIR/$target/release/lmx" "dist/$name/lmx"
```

with:

```yaml
                install -m 0755 "$CARGO_TARGET_DIR/$target/release/lmx" "dist/$name/lmx"
                install -m 0755 "$CARGO_TARGET_DIR/$target/release/lmxd" "dist/$name/lmxd"
```

---

## Task 6: Documentation

**Files:**
- Modify: `docs/contract.md`, `README.md`, `ARCHITECTURE.md`, `docs/plans/2026-10-06-guest-owner-design.md`, `docs/plans/2026-10-07-m2-lmxd-store-design.md`

The tables below are formatted by `task markdown/fix`; run it after editing, and `task ci/markdown-fmt` to check.

Replace `docs/contract.md`: `disk.unreadable`, the owner part of `lmx status`, and a section for `lmx store reserve`:

````markdown
# Host contract

The LimaNix host runs `sudo lmx <command> --json` over management SSH and reads one JSON answer from standard output.
This page defines contract version 1.

## Read one answer

Every `--json` answer is a single line with the same envelope:

```json
{"contract": 1, "ok": true, "data": {}}
{"contract": 1, "ok": false, "error": {"code": "disk.low", "message": "…", "details": {}}}
```

| Field      | Meaning                                                                                 |
| ---------- | --------------------------------------------------------------------------------------- |
| `contract` | Contract version. A host that does not know the version must not decode the rest.       |
| `ok`       | `true` selects `data`; `false` selects `error`.                                         |
| `data`     | Command-specific result.                                                                |
| `error`    | `code` for programs, `message` for people, optional `details` for code-specific values. |

A host decodes an answer in this order:

1. Read `contract`. If the version is unknown, stop: the rest cannot be decoded.
1. Read `ok`. An answer with `ok: true` and no `data`, or with `ok: false` and no `error`, is a protocol error.
1. Treat an unknown error `code` as a generic failure and show its `message`.
1. Keep `[]` and `null` apart: an empty list is a fact that was read, and `null` is a fact that was not.

A command that writes no answer, for example after a usage error or a lost connection, has failed.

## Interpret the exit status

| Status | Meaning                                                                                 |
| ------ | --------------------------------------------------------------------------------------- |
| `0`    | Success                                                                                 |
| `1`    | The operation failed; see the JSON answer, or standard error when no answer was written |
| `2`    | Usage error                                                                             |
| `3`    | The guest owner daemon is unavailable                                                   |
| `130`  | Cancelled                                                                               |

## Error codes

| Code                   | Meaning                                                            |
| ---------------------- | ------------------------------------------------------------------ |
| `owner.unavailable`    | `lmxd` is not reachable                                            |
| `apply.build_failed`   | `nixos-rebuild` failed for the requested generation                |
| `apply.cancelled`      | The operation was cancelled by an explicit request                 |
| `disk.low`             | Free bytes or inodes are below the platform minimum                |
| `disk.unreadable`      | The usage of the store file system cannot be read                  |
| `network.unreachable`  | A required destination, such as the binary cache, is unreachable   |
| `permission.denied`    | The caller is not allowed to run the operation                     |
| `generation.mismatch`  | The mounted inputs belong to a different generation than requested |
| `contract.unsupported` | The caller requested a contract version this binary does not speak |

`lmx status` and `lmx version` do not use these codes or the exit statuses `3` and `130`.
`lmx store reserve` uses exit status `3` when `lmxd` is unavailable.

## `lmx status`

`data` describes the guest. Every fact is optional: an unreadable fact is `null` and its reason is listed in `problems`.

| Field          | Meaning                                                                                                                   |
| -------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `generations`  | `desired` (mounted at `/mnt/limanix`), `built` (system profile), `booted` (running system)                                |
| `disk`         | `bytes`, `free_bytes` (including the root reserve), `available_bytes`, `inodes`, `free_inodes`                            |
| `interfaces`   | `name`, lowercase `mac` (`null` without a hardware address), and global-scope `ipv4` addresses of each interface          |
| `failed_units` | Names of failed systemd units                                                                                             |
| `owner`        | The guest owner daemon: `version`, `conditions` (`type`, `message`), `operations` (`task`, `kind`, `phase`, `created_at`) |
| `problems`     | `fact` and `message` for each fact that could not be read in full; omitted when empty                                     |

A problem's `fact` names the field that is `null` or incomplete, or is `config` when `/etc/lmx/config.json` cannot be read.
`owner` is `null` with an `owner` problem when `lmxd` does not answer within two seconds; the other facts are still answered.
Its conditions are computed when it is asked: `DiskLow` means less than the platform minimum of the store disk is free.
An operation's `created_at` is when it was requested, in Unix milliseconds.
Without the configuration, `ip` and `systemctl` are looked up in `PATH`, so a `config` problem marks a degraded answer.

A generation is `null` when its stage has no valid marker, for example on a system built before `lmx` existed.
A marker that exists but cannot be read also gives `null` and adds a `generations` problem.
The host compares the three generations to tell whether a build or a restart is still needed.

Examples: [complete](../contract/v1/status.json), [partial](../contract/v1/status-partial.json).

## `lmx store reserve`

Root only. The host runs it before it stops a running VM for an update.
`lmxd` collects unreferenced store paths when less than the collect threshold (20%) of the store disk is free, and waits for the collection; a collection already running is shared.
Interrupting the command does not stop the collection.

`data` has `before` and `after`, store disk usage objects as in `lmx status`, `freed_bytes`, and `collected`.

| Error code          | When                                                                                                                                                          |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `disk.low`          | Less than the platform minimum (10%) is still free afterwards; `details` has `before`, `after`, `freed_bytes`, and `collect_error` when the collection failed |
| `disk.unreadable`   | The usage of the store file system cannot be read                                                                                                             |
| `permission.denied` | The caller is not root                                                                                                                                        |
| `owner.unavailable` | `lmxd` cannot be reached; exit status `3`                                                                                                                     |

The host treats `disk.low` as a warning: the update may still succeed.

Examples: [enough room](../contract/v1/store-reserve.json), [disk low](../contract/v1/store-reserve-disk-low.json).

## `lmx version`

`data` has `version`, the release version of the binary, and `contract`, the contract version it speaks.

Example: [version](../contract/v1/version.json).

## Change the host contract

- Adding an optional field, a new command or a new error code is compatible and keeps the version.
  Hosts treat an unknown code as a generic failure.
- Renaming, removing or changing the meaning of a field needs a new contract version.
- Every version keeps its examples in `contract/v<version>/`; tests decode and re-encode them without loss.
- The LimaNix client pins an `lmx` release and tests its decoders against that release's examples.
````

Replace `README.md`: the daemon, its command and its boundaries:

````markdown
# lmx

[![License: Apache-2.0](https://img.shields.io/github/license/limanix/lmx?label=license)](LICENSE)

> **The owner of a LimaNix VM from inside: one command for people in the guest and one versioned contract for the host.**

`lmx` runs inside every [LimaNix](https://limanix.dev) guest.
It reports what the guest really is and answers the LimaNix host with versioned JSON over management SSH.
Its daemon, `lmxd`, owns the work that must not depend on a caller's session, starting with room in the Nix store.

[The problem](#the-problem) · [Commands](#commands) · [Host contract](docs/contract.md) · [Contributor map](ARCHITECTURE.md) · [Design](docs/plans/2026-10-06-guest-owner-design.md)

## The problem

A LimaNix VM is a long-lived development environment, but nothing inside the guest owned it.
The host reached in with literal commands over SSH and parsed their text.
Disk maintenance lived in four places, and the host could not tell which NixOS generation the guest had booted.

`lmx` gives the guest one owner and the host one contract:

```text
host ── ssh ──► sudo lmx <command> --json ──► {"contract": 1, "ok": true, "data": {…}}
person ───────► lmx <command>              ──► text for people
```

## Commands

| Command               | Answers or does                                                                                                |
| --------------------- | -------------------------------------------------------------------------------------------------------------- |
| `lmx help`            | The workspace and the commands inside the VM and on the Mac; also `lmx`, `lmx -h`                              |
| `lmx info`            | The kernel, guest disk, shared folders and failed units                                                        |
| `lmx welcome`         | The summary an interactive shell prints when it starts                                                         |
| `lmx status`          | Desired, built and booted generations; store disk usage; interfaces; failed systemd units; the state of `lmxd` |
| `lmx version`         | The binary version and the host contract version                                                               |
| `lmx store reserve`   | Collects unreferenced store paths when space is low; run in `lmxd`, root only                                  |
| `lmx clipboard copy`  | Copies standard input to the Mac clipboard                                                                     |
| `lmx clipboard paste` | Prints the Mac clipboard, if the terminal allows reads                                                         |
| `lmx session NAME`    | Opens a named session with the provider that the selected modules configure                                    |

Add `--json` to `status`, `version` and `store reserve` to answer with the [host contract](docs/contract.md).
`lmx status` reads every fact independently: an unreadable fact is reported as a problem, and the others are still answered.

Started under the name `pbcopy`, `pbpaste` or `limanix-session`, the binary keeps the arguments, messages and exit statuses of the shell command it replaces.

## The daemon

`lmxd` runs as root from systemd, started at boot and through `lmx.socket`, with readiness and a watchdog.
It replaces the platform's store guard, its timer, the daily `nix-gc` timer and the host's reserve over SSH:

- 5 minutes after boot and then every 15 minutes, it collects unreferenced store paths at idle priority when less than 20% of the store disk is free, and lists the garbage-collector roots in its log when less than 10% stays free;
- `lmx store reserve` does the same at once for the host and answers with the usage before and after;
- every operation is a Solti task, so its output reaches the journal (`journalctl -u lmx`).

Reference units are in [`packaging/systemd/`](packaging/systemd).

## Boundaries worth knowing early

- Nothing listens on the network. `lmxd` serves a Unix socket inside the guest, and the host reaches the guest only through management SSH.
- Facts are read in the caller's process with the caller's privileges and need no daemon.
- Owner operations run only in `lmxd`; without it they fail with exit status 3 instead of running in the caller.
- The clipboard travels through the terminal with OSC 52, or through tmux inside tmux; the terminal on the Mac must allow it.
- Text is colored only on a terminal, never with `NO_COLOR` or `TERM=dumb`.
- Configuration comes only from NixOS (`/etc/lmx/config.json`), never from the host at runtime.
- Release archives hold `lmx` and `lmxd`, static musl executables for `aarch64` and `x86_64` Linux.

## Development

Requirements: [Task](https://taskfile.dev/docs/installation) 3.53.1+, Git and Docker.
Tasks run Cargo in the [`ci/rust`](https://github.com/mr-chelyshkin/images) image, so CI and local checks use one toolchain.
Until Solti 0.0.7 is published, `lmxd` takes Solti from a local checkout by path (see `Cargo.toml`).
The tasks do not mount that checkout: run Cargo on the host, or mount it into the image at the same path, as the verification of the [M2 plan](docs/plans/2026-10-07-m2-lmxd-store.md) does.

| Task                   | Does                                                      |
| ---------------------- | --------------------------------------------------------- |
| `task ci/rust-fmt`     | Checks Rust formatting                                    |
| `task ci/rust-clippy`  | Lints every target and denies warnings                    |
| `task ci/rust-test`    | Runs unit and integration tests                           |
| `task ci/rust-docs`    | Builds API documentation and denies rustdoc warnings      |
| `task ci/rust-audit`   | Scans dependencies for advisories                         |
| `task ci/markdown-fmt` | Checks Markdown formatting                                |
| `task release/build`   | Builds `dist/lmx-<version>-<system>.tar.gz` and checksums |

Pull requests run every task in the table. `task ci` lists the checks.
`task rust/fix` and `task markdown/fix` apply the formatting that the checks expect.

Read the [contributor map](ARCHITECTURE.md) before changing a crate boundary or the host contract.

Licensed under [Apache 2.0](LICENSE).
````

Replace `ARCHITECTURE.md`: the new crates, boundaries and sources:

````markdown
# lmx contributor map

This document is the entry point for contributors and reviewers.
It explains what each crate owns, how the crates connect, and where to begin a change.

For usage, start with the [README](README.md) and the [host contract](docs/contract.md).
The [design](docs/plans/2026-10-06-guest-owner-design.md) records why the guest owner exists and what comes next.
Exact contracts live in the Rust source and its module-level documentation.

## Architecture at a glance

```text
NixOS ──► /etc/lmx/config.json ──► lmx-model::Config
                                        │
person or host ──► lmx (binary) ──► lmx-facts readers ──► statvfs, /proc, uname, ip, systemctl, markers
                         │
                         ├──► lmx-model::Envelope<T> ──► text or JSON on standard output
                         ├──► the caller's terminal or tmux, the session provider
                         └──► lmx-ipc ── gRPC, /run/lmx/lmx.sock ──► lmxd (root, systemd)
                                                                      ├──► store guard and reserve
                                                                      └──► Solti tasks ──► nix-store
```

`lmx-model` holds every value that crosses a boundary: the configuration written by NixOS and the answers read by the host.
`lmx-facts` reads the running system. The `lmx` binary parses the command line, combines facts, and renders them.
`lmxd` owns operations that outlive a caller; `lmx` asks for them through `lmx-ipc` and reports the answer.
Every program `lmxd` runs is a Solti task of a kind under `lmx.limanix.dev/v1`, executed by a private subprocess runner.
Caller commands, the welcome, the clipboard and sessions, depend on the caller's terminal and environment; the welcome also reads facts.

## Boundaries to preserve

- Values that cross a process boundary belong in `lmx-model`; a field added elsewhere is not part of any contract.
- `lmx-facts` performs reads only. It never changes the system and never needs a daemon.
- A reader that runs a program splits process I/O from a pure parser; tests cover the parser with fixed output.
- The host contract changes only as described in [Change the host contract](docs/contract.md#change-the-host-contract).
- Configuration comes from NixOS. The binaries never accept configuration from the host at runtime.
- Every crate forbids unsafe Rust with `#![forbid(unsafe_code)]`.
- Caller commands need the caller's terminal and environment, so they stay in the `lmx` process and never move into a daemon.
- Owner operations run only in `lmxd`. `lmx` never runs them itself when the daemon is unavailable.
- `lmx-ipc` and `lmx` do not depend on Solti; only `lmxd` does.
- `lmxd` runs programs only as Solti tasks, never with `std::process`: tools by their absolute paths in the configuration, and `/bin/sh` for the roots report.
- Only `lmxd` creates its tasks. The Task API on its socket reads them, and root may cancel or delete them.
- `pbcopy`, `pbpaste` and `limanix-session` keep the syntax of the shell commands they replaced; change them together with the platform.

## Source map

| Area              | Responsibility                                                 | Start here                                                                    |
| ----------------- | -------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| Contract types    | Configuration, envelope, error codes, status and version       | [`lmx-model/src/lib.rs`](crates/lmx-model/src/lib.rs)                         |
| Fact readers      | Disk, generations, machine, mounts, network and failed units   | [`lmx-facts/src/lib.rs`](crates/lmx-facts/src/lib.rs)                         |
| Command line      | Commands, other names, output selection and exit codes         | [`lmx/src/main.rs`](crates/lmx/src/main.rs)                                   |
| Status            | Collecting facts and rendering them                            | [`lmx/src/status.rs`](crates/lmx/src/status.rs)                               |
| Guest pages       | Help, info and the welcome for people in the guest             | [`lmx/src/welcome.rs`](crates/lmx/src/welcome.rs)                             |
| Terminal text     | Columns, wrapping and the palette                              | [`lmx/src/layout.rs`](crates/lmx/src/layout.rs)                               |
| Caller commands   | The clipboard through the terminal or tmux, and named sessions | [`lmx/src/clipboard.rs`](crates/lmx/src/clipboard.rs)                         |
| Owner calls       | `lmx store reserve` and the owner part of `lmx status`         | [`lmx/src/owner.rs`](crates/lmx/src/owner.rs)                                 |
| IPC               | The `lmx.v1.Owner` protocol and its Unix-socket client         | [`lmx-ipc/proto/lmx/v1/owner.proto`](crates/lmx-ipc/proto/lmx/v1/owner.proto) |
| Daemon            | Startup, serving the socket, systemd and the stopping order    | [`lmxd/src/main.rs`](crates/lmxd/src/main.rs)                                 |
| Store domain      | The guard, reserve, conditions and the store tasks             | [`lmxd/src/store.rs`](crates/lmxd/src/store.rs)                               |
| Contract examples | Published answers of each contract version                     | [`contract/v1/`](contract/v1)                                                 |

Files outside `crates/` provide executable context:

| Path                                      | Purpose                                                                          |
| ----------------------------------------- | -------------------------------------------------------------------------------- |
| [`crates/lmx/tests/`](crates/lmx/tests)   | The command-line contract against a prepared guest tree, with and without `lmxd` |
| [`packaging/systemd/`](packaging/systemd) | Reference units of `lmxd` for the platform                                       |
| [`Taskfile.yml`](Taskfile.yml)            | Checks and the release build                                                     |
| [`.github/workflows/`](.github/workflows) | Pull-request checks and tag releases                                             |

## Add a fact

1. Add the value to `Status` in `lmx-model` with a doc comment, and update the examples in `contract/v1/`.
1. Add a reader module to `lmx-facts`: an I/O function and a pure parser with fixture tests.
1. Collect it in `lmx/src/status.rs` with `record`, so a failure becomes a problem instead of an error.
1. Render it in the text output and extend `crates/lmx/tests/cli.rs`.

A fact that only a guest page shows, such as `mounts` and `machine`, skips steps 1 and 3.
A new optional field is a compatible change. Renaming or removing a field needs a new contract version.
````

In `docs/plans/2026-10-06-guest-owner-design.md`, the status:

```markdown
[the M1b plan](2026-10-06-m1b-lmx-user-surface.md). The S1 spike passed; its findings
for M2 are in [the S1 results](2026-10-07-s1-solti-spike.md).
```

with:

```markdown
[the M1b plan](2026-10-06-m1b-lmx-user-surface.md). The S1 spike passed; its findings
for M2 are in [the S1 results](2026-10-07-s1-solti-spike.md). M2 is implemented: `lmxd` keeps room
in the Nix store and answers `lmx store reserve`. See [the M2 design](2026-10-07-m2-lmxd-store-design.md)
and [the M2 plan](2026-10-07-m2-lmxd-store.md).
```

In `docs/plans/2026-10-07-m2-lmxd-store-design.md`, the status:

```markdown
Status: design agreed on 2026-10-07. It refines M2 of
[the guest owner design](2026-10-06-guest-owner-design.md) (section 12) with the findings of
[S1](2026-10-07-s1-solti-spike.md).
```

with:

```markdown
Status: design agreed on 2026-10-07 and implemented; see [the M2 plan](2026-10-07-m2-lmxd-store.md).
It refines M2 of [the guest owner design](2026-10-06-guest-owner-design.md) (section 12) with the
findings of [S1](2026-10-07-s1-solti-spike.md).
```

and the operation's time, which the implementation takes from the task's creation:

```markdown
  `{version, conditions: [{type, message}], operations: [{task, kind, phase, started_at}]}`.
```

with:

```markdown
  `{version, conditions: [{type, message}], operations: [{task, kind, phase, created_at}]}`.
  `created_at` is when the operation was requested, in Unix milliseconds.
```

The tests as built, in its Testing section:

```markdown
  - the reserve and guard decisions;
  - the first tick, from uptime;
  - proto conversions;
  - the authorization matrix.
```

with:

```markdown
  - the first tick, from uptime;
  - the commands of the task kinds, and the roots filter with the real `grep`;
  - configuration checks at startup;
  - proto conversions;
  - the authorization matrix.
```

```markdown
  1. two concurrent reserves below 20% run one collection, and both succeed;
  2. a reserve that stays below 10% returns `disk.low` and runs `StoreRoots`;
  3. `lmx status` works with a running `lmxd` and without one.
```

with:

```markdown
  1. two concurrent reserves below 20% run one collection, and both succeed;
  2. a reserve that stays below 10% returns `disk.low` and runs `StoreRoots`;
  3. the guard, on a low disk, collects through `nice` and `ionice`;
  4. `lmx status` works with a running `lmxd` and without one, and `lmx store reserve` without one
     answers `owner.unavailable` with exit status 3.
```

---

## Verification

**Linux.** The CI image cannot see the SDK path, so mount it at the same path. From the host, with `R=/Users/igoss/Desktop/lima-personal-shared/limanix/lmx` and `SD=/Users/igoss/Desktop/projects/solti/sdk`:

```bash
docker run --rm -v "$SD:$SD:ro" -v "$R:$R:ro" -w "$R" -v "$PWD/target-linux:/target" \
  -e CARGO_TARGET_DIR=/target -e RUSTUP_TOOLCHAIN=1.90.0 --entrypoint sh ghcr.io/mr-chelyshkin/ci/rust:1.90.0 -c '
    cargo fmt --all --check &&
    cargo clippy --workspace --all-targets --locked -- -D warnings &&
    cargo test --workspace --locked &&
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --locked &&
    for t in aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
      RUSTFLAGS="-D warnings" cargo build --release --locked --target $t -p lmx -p lmxd || exit 1
    done'
```

Expected: every step passes; `file` reports the four binaries as statically linked (aarch64) and static-pie (x86_64).

**Markdown.** `task ci/markdown-fmt` passes.

**Guest.** In the personal LimaNix VM, as in S1, with transient units only:

```bash
systemd-run --unit=lmx-m2 \
  --socket-property=ListenStream=/run/lmx/lmx.sock --socket-property=SocketMode=0666 \
  --property=Type=notify --property=WatchdogSec=30s --property=Restart=on-failure \
  --property=LimitCORE=0 /tmp/lmx-m2/lmxd --config /tmp/lmx-m2/config.json
```

With both thresholds of the test configuration at 99%: `lmx status --json` without the units reports `owner: null` with
its problem; with them, root and `nobody` read the owner, `nobody` gets `permission.denied` from `lmx store reserve`,
root gets `disk.low`, and the journal of the unit shows the guard's messages and the output of `nix-store --gc`. Remove
the units, `/tmp/lmx-m2` and `/run/lmx` afterwards.

## Suggested commits

The user commits. One commit per task keeps every commit building and green:
1. `feat(model): store reserve, owner state and lmxd tools in the contract`
1. `feat(ipc): lmx.v1.Owner protocol and Unix-socket client`
1. `feat(lmxd): guest owner daemon with the store domain`
1. `feat(lmx): store reserve and owner state through lmxd`
1. `build: reference systemd units and lmxd in the release archive`
1. `docs: M2 design, plan, contract and contributor map`
