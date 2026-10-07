# M3: apply and finalize in `lmxd` — implementation plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Move the guest side of an update into `lmxd`: `lmx apply` builds the mounted generation for the next boot, `lmx apply cancel` stops it, and after the restart `lmxd` checks the booted generation, finalizes it and reports `Converged`, which `lmx status --wait converged` waits for.

**Architecture:** As in M2, daemon logic orchestrates and every program runs as a Solti task of a kind under `lmx.limanix.dev/v1`: `SystemApply` (`nixos-rebuild boot`), `SystemHealth` and `SystemFinalize`. An apply lives in `lmxd`'s memory and streams its progress over the server-streaming RPC `Owner.Apply`; a tee on the runner's output publisher copies task output without loss. A generation observer in the system daemon checks health and finalizes; `Owner.Status` derives the generation conditions from the markers, the system profile and the last check.

**Tech Stack:** Rust 1.90.0 (edition 2024), Solti (local SDK by path until 0.0.7), tonic 0.14 with server streaming, tokio broadcast and watch channels, clap.

Design: [M3 design](2026-10-07-m3-apply-finalize-design.md). Background: [guest owner
design](2026-10-06-guest-owner-design.md), [M2 design](2026-10-07-m2-lmxd-store-design.md), [M2
plan](2026-10-07-m2-lmxd-store.md).

## Before you start

- **Repository:** `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx`, branch `feat/m1a-foundation`, with M2 committed (`6523ace`).
- **Never commit.** The user commits; the suggested commits are at the end.
- **Solti comes from the local SDK by path**, as in M2; the containerized `task ci/*` tasks do not see it. Run the host commands below, and the Linux check at the end with the SDK mounted.
- **Commands** run on the host with the pinned toolchain. Use your own `--target-dir` if an IDE builds the repository at the same time.
- **Every task ends green:** formatting, Clippy with `-D warnings` on every target, all tests, and rustdoc with `-D warnings`. Code arrives with its first caller, so no task leaves dead code.
- **Tests on macOS** start fake tools. A build's `PATH` is `/run/current-system/sw/bin`, which does not exist on a Mac, so the fake `nixos-rebuild` uses shell builtins and absolute paths only.

| Task | Delivers |
|---|---|
| 1 | Contract: configuration fields, error codes, condition names, apply answers and events |
| 2 | Facts: the generations of the system profile |
| 3 | The apply in `lmxd`: `Owner.Apply` and `Owner.CancelApply`, environment files, `SystemApply`, `--transient` |
| 4 | Health check, finalize and the generation conditions: the observer |
| 5 | `lmx apply`, `lmx apply cancel` and `lmx status --wait converged`, with integration tests |
| 6 | Documentation |

---

## Task 1: Contract for apply and finalize

**Files:**
- Create: `crates/lmx-model/src/apply.rs`, `contract/v1/apply-restart-required.json`, `contract/v1/apply-build-failed.json`, `contract/v1/apply-follow.jsonl`
- Modify: `crates/lmx-model/src/config.rs`, `crates/lmx-model/src/contract.rs`, `crates/lmx-model/src/owner.rs`, `crates/lmx-model/src/lib.rs`, `contract/v1/status.json`, `crates/lmxd/src/daemon.rs`, `crates/lmxd/src/tasks.rs`, `crates/lmx/src/help.rs`, `crates/lmx/tests/cli.rs`, `crates/lmx/tests/owner.rs`

Everything that crosses a process boundary lives in `lmx-model`. This task adds what M3 sends and reads: the
configuration fields of the transient daemon, three error codes, the generation condition names, and the answers and
events of `lmx apply`. Nothing uses them yet.

**Step 1: Write the failing tests of the configuration and the codes**

The platform renders the new fields; M1c does that. Here the configuration sample gains them. In
`crates/lmx-model/src/config.rs`, in the test `SAMPLE`, replace the user, disk and tools:

```rust
        "user": {"name": "dev", "home": "/home/dev", "uid": 501},
        "modules": ["lmx:console", "lmx:go"],
        "disk": {"collect_percent": 20, "minimum_percent": 10},
```

with:

```rust
        "user": {"name": "dev", "home": "/home/dev", "uid": 501, "gid": 100},
        "modules": ["lmx:console", "lmx:go"],
        "disk": {"collect_percent": 20, "minimum_percent": 10},
        "health": {"units": ["sshd.service", "lmx.socket"]},
```

and

```rust
            "grep": "/run/current-system/sw/bin/grep"
        }
    }"#;
```

with:

```rust
            "grep": "/run/current-system/sw/bin/grep",
            "nixos_rebuild": "/run/current-system/sw/bin/nixos-rebuild",
            "nix_env": "/run/current-system/sw/bin/nix-env",
            "sudo": "/run/wrappers/bin/sudo",
            "bash": "/run/current-system/sw/bin/bash",
            "systemd_run": "/run/current-system/sw/bin/systemd-run"
        }
    }"#;
```

In `crates/lmx-model/src/contract.rs`, the test of the wire names lists every code. Add the three new ones:

```rust
            (ErrorCode::ApplyCancelled, "apply.cancelled"),
            (ErrorCode::DiskLow, "disk.low"),
```

with:

```rust
            (ErrorCode::ApplyCancelled, "apply.cancelled"),
            (
                ErrorCode::ApplyEnvironmentFailed,
                "apply.environment_failed",
            ),
            (ErrorCode::DiskLow, "disk.low"),
```

and

```rust
            (ErrorCode::ContractUnsupported, "contract.unsupported"),
        ] {
```

with:

```rust
            (ErrorCode::ContractUnsupported, "contract.unsupported"),
            (ErrorCode::SystemDegraded, "system.degraded"),
            (ErrorCode::WaitTimeout, "wait.timeout"),
        ] {
```

**Step 2: Run the tests to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model`

Expected: the crate does not compile: ``no variant or associated item named `ApplyEnvironmentFailed` found for enum
`contract::ErrorCode` ``.

**Step 3: Add the configuration fields**

`user.gid` owns the environment files and is applied by number, so it works on the first create, before the account
exists. In `crates/lmx-model/src/config.rs`:

```rust
    /// Free-space thresholds of the guest disk.
    pub disk: DiskPolicy,
    /// Named-session provider selected by catalog modules.
```

with:

```rust
    /// Free-space thresholds of the guest disk.
    pub disk: DiskPolicy,
    /// Health check of an applied generation.
    pub health: Health,
    /// Named-session provider selected by catalog modules.
```

and

```rust
    /// Numeric user ID; equals the user's ID on the Mac.
    pub uid: u32,
}
```

with:

```rust
    /// Numeric user ID; equals the user's ID on the Mac.
    pub uid: u32,
    /// Numeric ID of the user's primary group, which owns the environment files.
    pub gid: u32,
}
```

and

```rust
    /// `grep`, used to leave the expected roots out of the garbage-collector roots report.
    pub grep: String,
}
```

with:

```rust
    /// `grep`, used to leave the expected roots out of the garbage-collector roots report.
    pub grep: String,
    /// `nixos-rebuild`, used to build a mounted generation for the next boot.
    pub nixos_rebuild: String,
    /// `nix-env`, used to remove older generations of the system profile.
    pub nix_env: String,
    /// `sudo`, used by the health check to run a command as the development account.
    pub sudo: String,
    /// `bash`, the login shell of that command.
    pub bash: String,
    /// `systemd-run`, used to run `switch-to-configuration` in its own unit, so stopping `lmxd`
    /// never interrupts a boot loader update.
    pub systemd_run: String,
}

/// Health check of an applied generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Health {
    /// Platform units that must be active, such as `sshd.service`.
    pub units: Vec<String>,
}
```

**Step 4: Add the error codes**

In `crates/lmx-model/src/contract.rs`, the variants:

```rust
    /// The operation was cancelled by an explicit request.
    ApplyCancelled,
    /// Free bytes or inodes are below the platform minimum.
```

with:

```rust
    /// The operation was cancelled by an explicit request.
    ApplyCancelled,
    /// The environment files of the generation could not be installed.
    ApplyEnvironmentFailed,
    /// Free bytes or inodes are below the platform minimum.
```

and

```rust
    /// The caller requested a contract version this binary does not speak.
    ContractUnsupported,
    /// A code this binary does not know, as received.
```

with:

```rust
    /// The caller requested a contract version this binary does not speak.
    ContractUnsupported,
    /// The booted generation failed its health check.
    SystemDegraded,
    /// A wait ended before its condition held.
    WaitTimeout,
    /// A code this binary does not know, as received.
```

their wire names in `as_str`:

```rust
            Self::ApplyCancelled => "apply.cancelled",
            Self::DiskLow => "disk.low",
```

with:

```rust
            Self::ApplyCancelled => "apply.cancelled",
            Self::ApplyEnvironmentFailed => "apply.environment_failed",
            Self::DiskLow => "disk.low",
```

and

```rust
            Self::ContractUnsupported => "contract.unsupported",
            Self::Other(code) => code,
```

with:

```rust
            Self::ContractUnsupported => "contract.unsupported",
            Self::SystemDegraded => "system.degraded",
            Self::WaitTimeout => "wait.timeout",
            Self::Other(code) => code,
```

and in `from_wire`:

```rust
            "apply.cancelled" => Self::ApplyCancelled,
            "disk.low" => Self::DiskLow,
```

with:

```rust
            "apply.cancelled" => Self::ApplyCancelled,
            "apply.environment_failed" => Self::ApplyEnvironmentFailed,
            "disk.low" => Self::DiskLow,
```

and

```rust
            "contract.unsupported" => Self::ContractUnsupported,
            other => Self::Other(other.to_owned()),
```

with:

```rust
            "contract.unsupported" => Self::ContractUnsupported,
            "system.degraded" => Self::SystemDegraded,
            "wait.timeout" => Self::WaitTimeout,
            other => Self::Other(other.to_owned()),
```

Export `Health` from `crates/lmx-model/src/lib.rs`:

```rust
pub use config::{
    CONFIG_PATH, CONFIG_SCHEMA, Config, ConfigError, DiskPolicy, Session, Tools, User, Vm,
};
```

with:

```rust
pub use config::{
    CONFIG_PATH, CONFIG_SCHEMA, Config, ConfigError, DiskPolicy, Health, Session, Tools, User, Vm,
};
```

**Step 5: Run the model tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model`

Expected: 15 tests pass.

**Step 6: Keep the other configurations complete**

Unknown and missing fields are packaging errors, so every configuration in the tests needs the new fields. In the test
configuration of `crates/lmxd/src/daemon.rs`:

```rust
            "user": {"name": "dev", "home": "/home/dev", "uid": 501},
            "modules": [],
            "disk": {"collect_percent": 20, "minimum_percent": 10},
```

with:

```rust
            "user": {"name": "dev", "home": "/home/dev", "uid": 501, "gid": 100},
            "modules": [],
            "disk": {"collect_percent": 20, "minimum_percent": 10},
            "health": {"units": ["sshd.service"]},
```

and

```rust
                "ionice": tool("ionice"),
                "grep": tool("grep")
            }
```

with:

```rust
                "ionice": tool("ionice"),
                "grep": tool("grep"),
                "nixos_rebuild": tool("nixos-rebuild"),
                "nix_env": tool("nix-env"),
                "sudo": "/run/wrappers/bin/sudo",
                "bash": tool("bash"),
                "systemd_run": tool("systemd-run")
            }
```

In the test tools of `crates/lmxd/src/tasks.rs`:

```rust
            grep: "/bin/grep".into(),
        }
    }
```

with:

```rust
            grep: "/bin/grep".into(),
            nixos_rebuild: "/bin/nixos-rebuild".into(),
            nix_env: "/bin/nix-env".into(),
            sudo: "/bin/sudo".into(),
            bash: "/bin/bash".into(),
            systemd_run: "/bin/systemd-run".into(),
        }
    }
```

In the test configuration of `crates/lmx/src/help.rs`:

```rust
    use lmx_model::{Config, DiskPolicy, Session, Tools, User, Vm};
```

with:

```rust
    use lmx_model::{Config, DiskPolicy, Health, Session, Tools, User, Vm};
```

and

```rust
                uid: 501,
            },
```

with:

```rust
                uid: 501,
                gid: 100,
            },
```

and

```rust
                minimum_percent: 10,
            },
            session: Session {
```

with:

```rust
                minimum_percent: 10,
            },
            health: Health { units: vec![] },
            session: Session {
```

and

```rust
                grep: "grep".into(),
            },
```

with:

```rust
                grep: "grep".into(),
                nixos_rebuild: "nixos-rebuild".into(),
                nix_env: "nix-env".into(),
                sudo: "sudo".into(),
                bash: "bash".into(),
                systemd_run: "systemd-run".into(),
            },
```

In the guest configuration of `crates/lmx/tests/cli.rs`:

```rust
            "user": {"name": "dev", "home": "/home/dev", "uid": 501},
            "modules": [],
            "disk": {"collect_percent": 20, "minimum_percent": 10},
```

with:

```rust
            "user": {"name": "dev", "home": "/home/dev", "uid": 501, "gid": 100},
            "modules": [],
            "disk": {"collect_percent": 20, "minimum_percent": 10},
            "health": {"units": ["sshd.service"]},
```

and

```rust
                "grep": "/run/current-system/sw/bin/grep"
            }
```

with:

```rust
                "grep": "/run/current-system/sw/bin/grep",
                "nixos_rebuild": "/run/current-system/sw/bin/nixos-rebuild",
                "nix_env": "/run/current-system/sw/bin/nix-env",
                "sudo": "/run/wrappers/bin/sudo",
                "bash": "/run/current-system/sw/bin/bash",
                "systemd_run": "/run/current-system/sw/bin/systemd-run"
            }
```

In `crates/lmx/tests/owner.rs`, the tools point into the guest tree, where Task 5 puts fakes, and `gid` is the tree's
group, which the test user may give the environment files Task 3 installs:

```rust
    os::unix::fs::PermissionsExt,
```

with:

```rust
    os::unix::fs::{MetadataExt, PermissionsExt},
```

and

```rust
            fs::set_permissions(&tool, fs::Permissions::from_mode(0o755))
                .expect("make a tool executable");
        }
```

with:

```rust
            fs::set_permissions(&tool, fs::Permissions::from_mode(0o755))
                .expect("make a tool executable");
        }
        // The environment files get the group of the tree, which the test user may give them.
        let gid = fs::metadata(root.path()).expect("tree metadata").gid();
```

and

```rust
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
```

with:

```rust
            "user": {"name": "dev", "home": "/home/dev", "uid": 501, "gid": gid},
            "modules": [],
            "disk": {"collect_percent": 20, "minimum_percent": 10},
            "health": {"units": ["sshd.service"]},
            "session": {"command": null, "providers": []},
            "tools": {
                "ip": path("bin/ip"),
                "systemctl": path("bin/systemctl"),
                "nix_store": path("bin/nix-store"),
                "nice": path("bin/nice"),
                "ionice": path("bin/ionice"),
                "grep": "/usr/bin/grep",
                "nixos_rebuild": path("bin/nixos-rebuild"),
                "nix_env": path("bin/nix-env"),
                "sudo": path("bin/sudo"),
                "bash": path("bin/bash"),
                "systemd_run": path("bin/systemd-run")
            }
```

**Step 7: Name the generation conditions**

`Owner.Status` reports the generation conditions next to `DiskLow`. In `crates/lmx-model/src/owner.rs`, after
`DISK_LOW`:



```rust
/// Name of the condition set while the mounted generation is not built: an apply is needed.
pub const OUT_OF_DATE: &str = "OutOfDate";

/// Name of the condition set while the built generation is not booted: a restart is needed.
pub const RESTART_REQUIRED: &str = "RestartRequired";

/// Name of the condition set when the mounted generation is built, booted, healthy and finalized: the
/// only generation of the system profile, with the boot entries rewritten.
pub const CONVERGED: &str = "Converged";

/// Name of the condition set while the booted generation fails its health check.
pub const DEGRADED: &str = "Degraded";
```

**Step 8: Publish the apply examples**

`lmx apply --follow --json` writes JSON Lines: events, then the envelope. A build that is done answers
`restart_required`; a failed build carries its exit status.

Create `contract/v1/apply-restart-required.json`:

```json
{
  "contract": 1,
  "ok": true,
  "data": {
    "generation": "0123456789ab",
    "state": "restart_required"
  }
}
```

Create `contract/v1/apply-build-failed.json`:

```json
{
  "contract": 1,
  "ok": false,
  "error": {
    "code": "apply.build_failed",
    "message": "nixos-rebuild failed with exit status 1.",
    "details": {
      "exit_code": 1
    }
  }
}
```

Create `contract/v1/apply-follow.jsonl`:

```json
{"event":"phase","phase":"environment"}
{"event":"phase","phase":"reserve"}
{"event":"warning","code":"disk.low","message":"Less than 10% of the guest disk is still free after collecting unreferenced store paths."}
{"event":"phase","phase":"build"}
{"event":"output","stream":"stderr","line":"building the system configuration..."}
{"event":"output","stream":"stderr","line":"these 12 derivations will be built:","truncated":true}
{"event":"lagged","skipped":12}
{"event":"output","stream":"stdout","line":"/nix/store/00000000000000000000000000000000-nixos-system-dev-box-26.05"}
{"contract":1,"ok":true,"data":{"generation":"0123456789ab","state":"restart_required"}}
```

The status example shows a generation condition. In `contract/v1/status.json`:

```json
      "conditions": [],
```

with:

```json
      "conditions": [
        {
          "type": "RestartRequired",
          "message": "Generation 0123456789ab is built; restart the VM to boot it."
        }
      ],
```

**Step 9: Add the apply answers and events**

Create `crates/lmx-model/src/apply.rs`. Its tests decode and re-encode the examples without loss, as for every published
answer:

```rust
//! Answers and events of `lmx apply`.

use serde::{Deserialize, Serialize};

use crate::{DiskUsage, ErrorCode};

/// Phase of an apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyPhase {
    /// Installing the environment files of the generation.
    Environment,
    /// Making room in the store.
    Reserve,
    /// Building the generation for the next boot.
    Build,
}

/// Stream of a line the build printed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// One event of `lmx apply --follow --json`; the contract envelope with the outcome follows the last
/// one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ApplyEvent {
    /// A phase started.
    Phase {
        /// The phase.
        phase: ApplyPhase,
    },
    /// A line the build printed.
    Output {
        /// Where the line was printed.
        stream: OutputStream,
        /// The line, without its newline; invalid UTF-8 is replaced.
        line: String,
        /// Whether the end of a long line was cut.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        truncated: bool,
    },
    /// A problem that does not stop the apply, such as a low disk.
    Warning {
        /// Code of the problem.
        code: ErrorCode,
        /// Explanation for people.
        message: String,
    },
    /// The follower fell behind and missed events.
    Lagged {
        /// Number of missed events.
        skipped: u64,
    },
}

/// State of an apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyState {
    /// The apply runs; follow it, or ask again later.
    Running,
    /// The generation is built for the next boot; restart the VM to boot it.
    RestartRequired,
}

/// Answer of `lmx apply`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Apply {
    /// The generation applied.
    pub generation: String,
    /// Where it stands.
    pub state: ApplyState,
}

/// Answer of `lmx apply cancel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelApply {
    /// Whether an apply of the generation was running and is now cancelled.
    pub cancelled: bool,
}

/// Details of an `apply.build_failed` failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildFailure {
    /// Exit status of `nixos-rebuild`, when it exited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Usage of the store disk, when the failure looks like a full disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk: Option<DiskUsage>,
}

#[cfg(test)]
mod tests {
    use crate::{Apply, ApplyEvent, BuildFailure, CONTRACT_VERSION, Envelope, ErrorCode};

    /// The published apply examples decode and encode without loss.
    #[test]
    fn contract_examples_round_trip() {
        let example = include_str!("../../../contract/v1/apply-restart-required.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<Apply> = serde_json::from_str(example).expect("example decodes");
        assert_eq!(envelope.contract, CONTRACT_VERSION);
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);

        let example = include_str!("../../../contract/v1/apply-build-failed.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<Apply> = serde_json::from_str(example).expect("example decodes");
        let error = envelope.error.clone().expect("a failure");
        assert_eq!(error.code, ErrorCode::ApplyBuildFailed);
        let failure: BuildFailure =
            serde_json::from_value(error.details.into()).expect("details are a build failure");
        assert_eq!(failure.exit_code, Some(1));
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);
    }

    /// Every line of the follow example but the last is an event; the last is the envelope.
    #[test]
    fn follow_example_is_events_then_an_envelope() {
        let example = include_str!("../../../contract/v1/apply-follow.jsonl");
        let lines: Vec<&str> = example.lines().collect();
        let (last, events) = lines.split_last().expect("lines");
        for line in events {
            let original: serde_json::Value = serde_json::from_str(line).expect("JSON");
            let event: ApplyEvent = serde_json::from_str(line).expect("an event");
            assert_eq!(serde_json::to_value(&event).expect("encode"), original);
        }
        let envelope: Envelope<Apply> = serde_json::from_str(last).expect("an envelope");
        assert!(envelope.ok);
    }
}
```

In `crates/lmx-model/src/lib.rs`, the table of the crate documentation:

```rust
//! | [`Reserve`]  | `lmx store reserve`                     | the host                |
```

with:

```rust
//! | [`Reserve`]  | `lmx store reserve`                     | the host                |
//! | [`Apply`]    | `lmx apply`, with [`ApplyEvent`]s       | the host and people     |
```

the module:

```rust
mod config;
mod contract;
```

with:

```rust
mod apply;
mod config;
mod contract;
```

and the exports:

```rust
pub use config::{
```

with:

```rust
pub use apply::{
    Apply, ApplyEvent, ApplyPhase, ApplyState, BuildFailure, CancelApply, OutputStream,
};
pub use config::{
```

and

```rust
pub use owner::{Condition, DISK_LOW, Operation, Owner};
```

with:

```rust
pub use owner::{
    CONVERGED, Condition, DEGRADED, DISK_LOW, OUT_OF_DATE, Operation, Owner, RESTART_REQUIRED,
};
```

**Step 10: Run the workspace tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml --workspace`

Expected: 129 tests pass, including `contract_examples_round_trip` and `follow_example_is_events_then_an_envelope`.

---

## Task 2: The generations of the system profile

**Files:**
- Modify: `crates/lmx-facts/src/generations.rs`

A booted generation is finalized when the system profile keeps no older generation. NixOS links each generation as
`/nix/var/nix/profiles/system-<number>-link`; counting them is a fact, read without a daemon.

**Step 1: Write the failing test**

In the tests of `crates/lmx-facts/src/generations.rs`, after `use super::*;`:



```rust
    #[test]
    fn counts_only_the_generation_links_of_the_system_profile() {
        let profiles = tempfile::tempdir().expect("temporary directory");
        for name in [
            "system-1-link",
            "system-12-link",
            "system-foo-link",
            "system",
            "default-3-link",
        ] {
            fs::write(profiles.path().join(name), "").expect("write an entry");
        }
        fs::create_dir(profiles.path().join("per-user")).expect("create a directory");
        assert_eq!(system_generations(profiles.path()).expect("readable"), 2);
    }
```

**Step 2: Run it to see it fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts generations`

Expected: ``cannot find function `system_generations` in this scope``.

**Step 3: Count the generations**

Before the tests:



```rust
/// Directory holding the system profile and its generation links.
pub const PROFILES_PATH: &str = "/nix/var/nix/profiles";

/// Counts the generations of the system profile: the `system-<number>-link` entries of `profiles`.
///
/// More than one means older generations are kept, so the current one is not finalized yet.
pub fn system_generations(profiles: &Path) -> Result<usize, FactError> {
    let unreadable = |source| FactError::Io {
        what: "the system profile generations",
        source,
    };
    let mut count = 0;
    for entry in fs::read_dir(profiles).map_err(unreadable)? {
        if is_generation(&entry.map_err(unreadable)?.file_name()) {
            count += 1;
        }
    }
    Ok(count)
}

/// Whether `name` is a `system-<number>-link`.
fn is_generation(name: &OsStr) -> bool {
    name.to_str()
        .and_then(|name| name.strip_prefix("system-")?.strip_suffix("-link"))
        .is_some_and(|number| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        })
}
```

and import `OsStr`:

```rust
use std::{
    fs, io,
```

with:

```rust
use std::{
    ffi::OsStr,
    fs, io,
```

**Step 4: Run the facts tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts`

Expected: 28 tests pass.

---

## Task 3: The apply in `lmxd`

**Files:**
- Create: `crates/lmxd/src/paths.rs`, `crates/lmxd/src/launch.rs`, `crates/lmxd/src/capture.rs`, `crates/lmxd/src/environment.rs`, `crates/lmxd/src/apply.rs`
- Modify: `crates/lmx-ipc/proto/lmx/v1/owner.proto`, `crates/lmx-ipc/src/convert.rs`, `crates/lmx-ipc/src/lib.rs`, `crates/lmxd/src/tasks.rs`, `crates/lmxd/src/store.rs`, `crates/lmxd/src/owner.rs`, `crates/lmxd/src/daemon.rs`, `crates/lmxd/src/main.rs`, `crates/lmxd/src/lib.rs`, `crates/lmx/tests/owner.rs`

This task moves the host's guest steps of an update into `lmxd`. An apply installs the generation's environment files,
reserves room in the store and runs `nixos-rebuild boot` as a `SystemApply` task. It belongs to `lmxd`: its state lives
in memory, followers join and leave, and a cancel stops it. The protocol and the daemon change together, because the
generated server trait gains the new calls.

**Step 1: Declare the calls**

In `crates/lmx-ipc/proto/lmx/v1/owner.proto`, the service:

```protobuf
  rpc Reserve(ReserveRequest) returns (ReserveResponse);
}
```

with:

```protobuf
  rpc Reserve(ReserveRequest) returns (ReserveResponse);
  // Builds the mounted generation for the next boot and streams its progress. Root only.
  rpc Apply(ApplyRequest) returns (stream ApplyEvent);
  // Cancels the apply of a generation. Root only.
  rpc CancelApply(CancelApplyRequest) returns (CancelApplyResponse);
}
```

and at the end of the file, the messages. A phase travels as its wire name; the last event is the outcome, or, without
`follow`, the state `running`:

```protobuf
// Input of `Apply`.
message ApplyRequest {
  // Generation to apply; it must be the one mounted at `/mnt/limanix`.
  string generation = 1;
  // Whether to stream the progress; without it the answer comes once the apply runs.
  bool follow = 2;
}

// One event of `Apply`; the last one is the outcome.
message ApplyEvent {
  // What happened.
  oneof event {
    // A phase started: `environment`, `reserve` or `build`.
    string phase = 1;
    // A line the build printed.
    OutputLine output = 2;
    // A problem that does not stop the apply, such as `disk.low`.
    Failure warning = 3;
    // Number of events the follower missed.
    uint64 lagged = 4;
    // How the apply ended, or that it runs when it is not followed.
    ApplyOutcome outcome = 5;
  }
}

// A line the build printed.
message OutputLine {
  // Whether the line came from standard error.
  bool stderr = 1;
  // The line, without its newline.
  bytes line = 2;
  // Whether the end of a long line was cut.
  bool truncated = 3;
}

// How an apply ended.
message ApplyOutcome {
  // A result, or a failure such as `apply.build_failed`.
  oneof outcome {
    // The generation and its state.
    ApplyResult result = 1;
    // The apply failed.
    Failure failure = 2;
  }
}

// The generation of an apply and its state.
message ApplyResult {
  // The generation applied.
  string generation = 1;
  // `running` or `restart_required`.
  string state = 2;
}

// Input of `CancelApply`.
message CancelApplyRequest {
  // Generation whose apply to cancel.
  string generation = 1;
}

// Output of `CancelApply`.
message CancelApplyResponse {
  // Whether an apply was cancelled, or a failure such as `permission.denied`.
  oneof outcome {
    // Whether an apply of the generation was running and is now cancelled.
    bool cancelled = 1;
    // The cancel failed.
    Failure failure = 2;
  }
}
```

**Step 2: Write the failing test of the conversions**

In the tests of `crates/lmx-ipc/src/convert.rs`, before `rejects_an_answer_without_an_outcome`:



```rust
    #[test]
    fn apply_events_and_outcomes_survive_the_wire() {
        for event in [
            ApplyEvent::Phase {
                phase: ApplyPhase::Build,
            },
            ApplyEvent::Output {
                stream: OutputStream::Stderr,
                line: "building the system configuration...".into(),
                truncated: true,
            },
            ApplyEvent::Warning {
                code: ErrorCode::DiskLow,
                message: "full".into(),
            },
            ApplyEvent::Lagged { skipped: 3 },
        ] {
            assert_eq!(
                apply_message(event.clone().into()),
                Ok(ApplyMessage::Event(event))
            );
        }
        let applied = Apply {
            generation: "0123456789ab".into(),
            state: ApplyState::RestartRequired,
        };
        assert_eq!(
            apply_message(outcome_event(Ok(applied.clone()))),
            Ok(ApplyMessage::Outcome(Ok(applied)))
        );
    }
```

**Step 3: Run it to see it fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-ipc`

Expected: the tests do not compile: the apply types and ``apply_message`` are unknown.

**Step 4: Convert apply events and outcomes**

In `crates/lmx-ipc/src/convert.rs`, the imports:

```rust
use lmx_model::{Condition, DiskUsage, ErrorBody, ErrorCode, Operation, Owner, Reserve};
```

with:

```rust
use lmx_model::{
    Apply, ApplyEvent, ApplyPhase, ApplyState, CancelApply, Condition, DiskUsage, ErrorBody,
    ErrorCode, Operation, OutputStream, Owner, Reserve,
};
```

and after `reserve_outcome`, the conversions. The server turns events into messages; the client reads each message back
as an event or the outcome:

```rust
/// Wire name of an apply phase.
const fn phase_name(phase: ApplyPhase) -> &'static str {
    match phase {
        ApplyPhase::Environment => "environment",
        ApplyPhase::Reserve => "reserve",
        ApplyPhase::Build => "build",
    }
}

/// Apply phase with the wire name `name`.
fn phase(name: &str) -> Option<ApplyPhase> {
    [
        ApplyPhase::Environment,
        ApplyPhase::Reserve,
        ApplyPhase::Build,
    ]
    .into_iter()
    .find(|phase| phase_name(*phase) == name)
}

/// Wire name of an apply state.
const fn state_name(state: ApplyState) -> &'static str {
    match state {
        ApplyState::Running => "running",
        ApplyState::RestartRequired => "restart_required",
    }
}

/// Apply state with the wire name `name`.
fn state(name: &str) -> Option<ApplyState> {
    [ApplyState::Running, ApplyState::RestartRequired]
        .into_iter()
        .find(|state| state_name(*state) == name)
}

impl From<ApplyEvent> for proto::ApplyEvent {
    fn from(event: ApplyEvent) -> Self {
        use proto::apply_event::Event;
        let event = match event {
            ApplyEvent::Phase { phase } => Event::Phase(phase_name(phase).to_owned()),
            ApplyEvent::Output {
                stream,
                line,
                truncated,
            } => Event::Output(proto::OutputLine {
                stderr: stream == OutputStream::Stderr,
                line: line.into_bytes(),
                truncated,
            }),
            ApplyEvent::Warning { code, message } => Event::Warning(
                ErrorBody {
                    code,
                    message,
                    details: Map::new(),
                }
                .into(),
            ),
            ApplyEvent::Lagged { skipped } => Event::Lagged(skipped),
        };
        Self { event: Some(event) }
    }
}

/// The last event of an apply: how it ended, or that it runs.
pub fn outcome_event(outcome: Result<Apply, ErrorBody>) -> proto::ApplyEvent {
    let outcome = match outcome {
        Ok(apply) => proto::apply_outcome::Outcome::Result(proto::ApplyResult {
            generation: apply.generation,
            state: state_name(apply.state).to_owned(),
        }),
        Err(error) => proto::apply_outcome::Outcome::Failure(error.into()),
    };
    proto::ApplyEvent {
        event: Some(proto::apply_event::Event::Outcome(proto::ApplyOutcome {
            outcome: Some(outcome),
        })),
    }
}

/// One `Apply` event as the client reads it.
#[derive(Debug, PartialEq, Eq)]
pub enum ApplyMessage {
    /// Progress of the apply.
    Event(ApplyEvent),
    /// How the apply ended, or that it runs; nothing follows.
    Outcome(Result<Apply, ErrorBody>),
}

/// Reads one `Apply` event.
pub fn apply_message(event: proto::ApplyEvent) -> Result<ApplyMessage, InvalidAnswer> {
    use proto::apply_event::Event;
    Ok(match event.event.ok_or(InvalidAnswer("an apply event"))? {
        Event::Phase(name) => ApplyMessage::Event(ApplyEvent::Phase {
            phase: phase(&name).ok_or(InvalidAnswer("a known apply phase"))?,
        }),
        Event::Output(output) => ApplyMessage::Event(ApplyEvent::Output {
            stream: if output.stderr {
                OutputStream::Stderr
            } else {
                OutputStream::Stdout
            },
            line: String::from_utf8_lossy(&output.line).into_owned(),
            truncated: output.truncated,
        }),
        Event::Warning(failure) => {
            let warning = ErrorBody::from(failure);
            ApplyMessage::Event(ApplyEvent::Warning {
                code: warning.code,
                message: warning.message,
            })
        }
        Event::Lagged(skipped) => ApplyMessage::Event(ApplyEvent::Lagged { skipped }),
        Event::Outcome(outcome) => {
            ApplyMessage::Outcome(match outcome.outcome.ok_or(InvalidAnswer("an outcome"))? {
                proto::apply_outcome::Outcome::Result(result) => Ok(Apply {
                    state: state(&result.state).ok_or(InvalidAnswer("a known apply state"))?,
                    generation: result.generation,
                }),
                proto::apply_outcome::Outcome::Failure(failure) => Err(failure.into()),
            })
        }
    })
}

/// Outcome of a `CancelApply` call: the answer, or the failure the contract names.
pub fn cancel_outcome(
    response: proto::CancelApplyResponse,
) -> Result<Result<CancelApply, ErrorBody>, InvalidAnswer> {
    match response.outcome {
        Some(proto::cancel_apply_response::Outcome::Cancelled(cancelled)) => {
            Ok(Ok(CancelApply { cancelled }))
        }
        Some(proto::cancel_apply_response::Outcome::Failure(failure)) => Ok(Err(failure.into())),
        None => Err(InvalidAnswer("an outcome")),
    }
}
```

Export them from `crates/lmx-ipc/src/lib.rs`:

```rust
pub use convert::{InvalidAnswer, reserve_outcome};
```

with:

```rust
pub use convert::{
    ApplyMessage, InvalidAnswer, apply_message, cancel_outcome, outcome_event, reserve_outcome,
};
```

**Step 5: Run the protocol tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-ipc`

Expected: 4 tests pass. `lmxd` does not compile until its service implements the new calls below.

**Step 6: Name the guest paths below a system root**

Tests run the daemon on a prepared tree, so every guest path `lmxd` reads or writes besides the store and the socket
hangs off one root. Create `crates/lmxd/src/paths.rs`:

```rust
//! Guest locations that `lmxd` reads and writes, below a system root.

use std::path::PathBuf;

use lmx_facts::generations::{GenerationPaths, PROFILES_PATH};

/// Guest locations below a system root: `/` in a guest, a prepared tree in tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Paths {
    /// The system root.
    root: PathBuf,
}

impl Paths {
    /// Locations below `root`.
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// `path`, an absolute guest path, below the root.
    fn at(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    /// Inputs of the mounted generation.
    pub(crate) fn mount(&self) -> PathBuf {
        self.at("/mnt/limanix")
    }

    /// Flake reference of the mounted generation's system.
    pub(crate) fn flake(&self) -> String {
        format!("path:{}#runtime", self.mount().join("flake").display())
    }

    /// Directory of the installed environment files.
    pub(crate) fn environment(&self) -> PathBuf {
        self.at("/etc/limanix")
    }

    /// Generation markers.
    pub(crate) fn generations(&self) -> GenerationPaths {
        GenerationPaths::under(&self.root)
    }

    /// Directory of the system profile and its generations.
    pub(crate) fn profiles(&self) -> PathBuf {
        self.at(PROFILES_PATH)
    }

    /// The system profile.
    pub(crate) fn system_profile(&self) -> PathBuf {
        self.profiles().join("system")
    }
}
```

**Step 7: Start tasks and wait for them in one place**

Store, apply and, in Task 4, the observer start tasks and wait for them. Move M2's code from the store into
`crates/lmxd/src/launch.rs`; a task's slot, admission and timeout become a `Placement`, and a failure keeps its phase
and exit status:

```rust
//! Starting `lmxd` tasks and waiting for them.

use std::time::Duration;

use solti::{
    core::SupervisorApi,
    model::{
        AdmissionPolicy, RestartPolicy, TaskId, TaskManifest, TaskPhase, TaskSpec, TaskStatus,
        TaskWorkload,
    },
};

/// How often a waiting caller checks its task.
const POLL: Duration = Duration::from_millis(100);

/// Where and how long a task runs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placement {
    /// Slot of the task.
    pub(crate) slot: &'static str,
    /// What happens when the slot is busy.
    pub(crate) admission: AdmissionPolicy,
    /// Longest an attempt may run; [`Duration::MAX`] for no limit.
    pub(crate) timeout: Duration,
}

/// How a task ended without success.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ended {
    /// Phase it ended in, such as `exhausted` or `canceled`.
    pub(crate) phase: TaskPhase,
    /// Exit status of its process, when the process exited.
    pub(crate) exit_code: Option<i32>,
    /// Why, for people.
    pub(crate) message: String,
}

/// Creates the task `name` that runs `workload` once.
pub(crate) async fn start(
    supervisor: &SupervisorApi,
    name: &str,
    workload: TaskWorkload,
    placement: Placement,
) -> Result<TaskId, String> {
    let timeout = u64::try_from(placement.timeout.as_millis()).unwrap_or(u64::MAX);
    let spec = TaskSpec::builder(placement.slot, workload, timeout)
        .restart(RestartPolicy::Never)
        .admission(placement.admission)
        .build()
        .map_err(|error| error.to_string())?;
    let manifest = TaskManifest::new(name, spec).map_err(|error| error.to_string())?;
    let task = supervisor
        .create_task(manifest)
        .await
        .map_err(|error| error.to_string())?;
    Ok(task.name().clone())
}

/// Whether a task with `status` will still run: pending or running, and built by its runner.
///
/// A task whose runner could not build it stays pending for good, so it does not run.
pub(crate) fn runs(status: &TaskStatus) -> bool {
    status.phase().is_active() && !status.reconciliation_failed()
}

/// Waits until the task `name` ends; an outcome other than success is [`Ended`].
pub(crate) async fn finished(supervisor: &SupervisorApi, name: &TaskId) -> Result<(), Ended> {
    let mut poll = tokio::time::interval(POLL);
    loop {
        poll.tick().await;
        let Some(task) = supervisor.get_task(name) else {
            return Err(Ended {
                phase: TaskPhase::Canceled,
                exit_code: None,
                message: format!("task {name} was removed"),
            });
        };
        let status = task.status();
        if status.reconciliation_failed() {
            return Err(Ended {
                phase: status.phase(),
                exit_code: None,
                message: status.reconciled().message().to_owned(),
            });
        }
        let phase = status.phase();
        if phase == TaskPhase::Succeeded {
            return Ok(());
        }
        if phase.is_terminal() {
            return Err(Ended {
                phase,
                exit_code: status.exit_code(),
                message: status
                    .error()
                    .map_or_else(|| format!("task {name} ended {phase}"), ToOwned::to_owned),
            });
        }
    }
}
```

In `crates/lmxd/src/store.rs`, the imports:

```rust
    model::{
        AdmissionPolicy, RestartPolicy, TaskId, TaskManifest, TaskPhase, TaskSpec, TaskStatus,
        TaskWorkload,
    },
};
use tokio::sync::Mutex;

use crate::tasks::{self, Kind, Priority};
```

with:

```rust
    model::{AdmissionPolicy, TaskId, TaskWorkload},
};
use tokio::sync::Mutex;

use crate::{
    launch::{self, Placement},
    tasks::{self, Kind, Priority},
};
```

Remove `POLL`:

```rust
/// How often a waiting caller checks its task.
const POLL: Duration = Duration::from_millis(100);
```

and the end of `start`, `active`, `finished` and `runs`:

```rust
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
```

with:

```rust
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
```

A failed build is explained as a full disk when usage is below the minimum or the build said so, as the host does today.
Before `collect`:



```rust
    /// Usage of the store disk, when it is below the platform minimum or `full` says the disk ran
    /// out, so a failure can be explained as a full disk.
    pub(crate) fn shortage(&self, full: bool) -> Option<DiskUsage> {
        (self.usage)()
            .ok()
            .filter(|usage| full || usage.below(self.policy.minimum_percent))
    }
```

**Step 8: Copy task output for `lmxd`**

Solti's output stream is live and starts when a subscriber arrives; an apply follower and a health-check reason need
every line. Create `crates/lmxd/src/capture.rs`, a tee on the runner's output publisher. Registering before the task
exists loses nothing:

```rust
//! Copies of task output for `lmxd` itself.
//!
//! Solti's output stream is live: it begins when a subscriber arrives and misses what came before.
//! `lmxd` needs every line of some of its tasks, such as the build its apply followers read, or the
//! reason a health check failed. [`Tee`] wraps the publisher the runner writes to and copies the
//! lines of the tasks someone listens to; registering before the task exists loses nothing.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use solti::{
    model::{StreamKind, TaskId},
    runner::{
        OutputChunkRef, OutputPublisher, OutputPublisherHandle, OutputSink, request_output_sink,
    },
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// One line a task printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    /// Whether it came from standard error.
    pub(crate) stderr: bool,
    /// The line; invalid UTF-8 is replaced.
    pub(crate) text: String,
    /// Whether the end of a long line was cut.
    pub(crate) truncated: bool,
}

/// Listeners of task output, by task name.
#[derive(Debug, Default)]
pub(crate) struct Capture {
    /// Where the lines of each listened-to task go.
    listeners: Mutex<HashMap<String, UnboundedSender<Line>>>,
}

impl Capture {
    /// Receives the output of the task `name` from now on; call it before the task is created.
    pub(crate) fn listen(&self, name: &str) -> UnboundedReceiver<Line> {
        let (sender, receiver) = unbounded_channel();
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), sender);
        receiver
    }

    /// Stops copying the output of the task `name`.
    pub(crate) fn forget(&self, name: &str) {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
    }

    /// Where the lines of the task `name` go, if someone listens.
    fn listener(&self, name: &TaskId) -> Option<UnboundedSender<Line>> {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name.as_str())
            .cloned()
    }
}

/// Output publisher that passes every line on and copies the lines of listened-to tasks.
pub(crate) struct Tee {
    /// The publisher the supervisor gave the runner.
    pub(crate) inner: OutputPublisherHandle,
    /// Listeners of task output.
    pub(crate) capture: Arc<Capture>,
}

impl OutputPublisher for Tee {
    fn sink_for(&self, task_name: &TaskId, generation: u64, attempt: u32) -> Option<OutputSink> {
        let inner = request_output_sink(&self.inner, task_name, generation, attempt);
        let Some(listener) = self.capture.listener(task_name) else {
            return inner;
        };
        Some(OutputSink::new_borrowed(
            generation,
            attempt,
            move |chunk: OutputChunkRef<'_>| {
                let stderr = chunk.stream() == StreamKind::Stderr;
                if let Some(inner) = &inner {
                    match (stderr, chunk.truncated()) {
                        (false, false) => inner.stdout_line_bytes(chunk.line()),
                        (false, true) => inner.stdout_line_bytes_truncated(chunk.line()),
                        (true, false) => inner.stderr_line_bytes(chunk.line()),
                        (true, true) => inner.stderr_line_bytes_truncated(chunk.line()),
                    }
                }
                let _ = listener.send(Line {
                    stderr,
                    text: String::from_utf8_lossy(chunk.line()).into_owned(),
                    truncated: chunk.truncated(),
                });
            },
        ))
    }
}
```

**Step 9: Install the environment files**

The host writes the user's `[env]` next to the generation's flake. Create `crates/lmxd/src/environment.rs`: it installs
the files atomically with mode `0640` and the development account's group, and parses `environment` for the build's
variables:

```rust
//! Environment files of a generation.
//!
//! The host writes the user's `[env]` into two files next to the generation's flake, so the values
//! stay out of the flake and the Nix store. Before a build they are installed into `/etc/limanix`,
//! where every service reads `environment` through a systemd drop-in and login shells source
//! `environment.sh`. Only root and the development account's group may read them.

use std::{
    fs::{self, OpenOptions, Permissions},
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt, chown},
};

use crate::paths::Paths;

/// Names of the environment files, the same in the generation and in `/etc/limanix`.
const FILES: [&str; 2] = ["environment", "environment.sh"];

/// Mode of the installed files: root writes, the group reads.
const FILE_MODE: u32 = 0o640;

/// Installs the generation's environment files into `/etc/limanix`.
///
/// The directory gets mode `0755`. Each file is written beside its target with mode `0640` and the
/// group `gid`, then renamed over it, so a reader never sees half a file.
pub(crate) fn install(paths: &Paths, gid: u32) -> io::Result<()> {
    let directory = paths.environment();
    fs::create_dir_all(&directory)?;
    fs::set_permissions(&directory, Permissions::from_mode(0o755))?;
    for name in FILES {
        let content = fs::read(paths.mount().join(name))?;
        let staged = directory.join(format!(".{name}.new"));
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(FILE_MODE)
            .open(&staged)?;
        file.write_all(&content)?;
        file.sync_all()?;
        // The creation mode is narrowed by the umask; set it exactly.
        fs::set_permissions(&staged, Permissions::from_mode(FILE_MODE))?;
        chown(&staged, None, Some(gid))?;
        fs::rename(&staged, directory.join(name))?;
    }
    Ok(())
}

/// Variables of an installed `environment` file, in file order.
///
/// The host writes one `NAME="value"` assignment per variable, escaping `\`, `"`, `$` and the
/// backtick with a backslash; a value may span lines. Anything else ends the parse, keeping the
/// variables read so far.
pub(crate) fn variables(text: &str) -> Vec<(String, String)> {
    let mut variables = Vec::new();
    let mut rest = text;
    loop {
        rest = rest.trim_start_matches(['\n', '\r']);
        let Some((name, after)) = rest.split_once("=\"") else {
            return variables;
        };
        if name.is_empty() || name.contains(['\n', '"', '=']) {
            return variables;
        }
        let mut value = String::new();
        let mut characters = after.char_indices();
        let end = loop {
            match characters.next() {
                Some((_, '\\')) => match characters.next() {
                    Some((_, escaped)) => value.push(escaped),
                    None => return variables,
                },
                Some((index, '"')) => break index,
                Some((_, character)) => value.push(character),
                None => return variables,
            }
        };
        variables.push((name.to_owned(), value));
        rest = &after[end + 1..];
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;

    #[test]
    fn reads_the_assignments_the_host_writes() {
        let text = "HTTP_PROXY=\"http://proxy:3128\"\nGREETING=\"say \\\"hi\\\"\nto \\$USER\"\n";
        assert_eq!(
            variables(text),
            [
                ("HTTP_PROXY".to_owned(), "http://proxy:3128".to_owned()),
                ("GREETING".to_owned(), "say \"hi\"\nto $USER".to_owned()),
            ]
        );
        assert!(variables("").is_empty());
    }

    #[test]
    fn installs_the_files_for_root_and_the_group_only() {
        let root = tempfile::tempdir().expect("temporary root");
        let paths = Paths::new(root.path().to_path_buf());
        fs::create_dir_all(paths.mount()).expect("create the mount");
        fs::write(paths.mount().join("environment"), "A=\"1\"\n").expect("write");
        fs::write(paths.mount().join("environment.sh"), "export A=\"1\"\n").expect("write");
        let gid = fs::metadata(root.path()).expect("metadata").gid();

        install(&paths, gid).expect("install");
        for name in FILES {
            let installed = paths.environment().join(name);
            let metadata = fs::metadata(&installed).expect("installed");
            assert_eq!(metadata.mode() & 0o777, 0o640, "{name}");
            assert_eq!(metadata.gid(), gid, "{name}");
        }
        assert_eq!(
            fs::read_to_string(paths.environment().join("environment")).expect("read"),
            "A=\"1\"\n"
        );
    }
}
```

**Step 10: Add the system task kinds**

Replace `crates/lmxd/src/tasks.rs`. The runner now turns a task into a `Process` with an environment, and wraps the
runner's output publisher with the tee. Every system task gets `PATH=/run/current-system/sw/bin`; `SystemApply` adds the
user's variables on top, as systemd's `EnvironmentFile` gives them today. `SystemHealth` and `SystemFinalize` are
scripts with absolute tools; finalize runs `switch-to-configuration` in its own unit through `systemd-run`, as
`nixos-rebuild` does, so no cancel or stop interrupts a boot loader update. Task 4 starts them:

```rust
//! Workload kinds of `lmxd` and the runner that executes them.
//!
//! Every operation of `lmxd` is a Solti task whose kind lives under [`API_VERSION`], so the Task API
//! lists it, streams its output and keeps its runs. Each kind runs one program. The runner turns
//! such a task into a `solti.io/v1` subprocess task and builds it with a private subprocess runner,
//! which clears the environment, owns the process group and captures the output. The private runner
//! is not registered with the supervisor, so no API caller can start an arbitrary program as root.

use std::{fs, sync::Arc};

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

use crate::{
    capture::{Capture, Tee},
    environment,
    paths::Paths,
};

/// API version of the workload kinds of `lmxd`.
pub(crate) const API_VERSION: &str = "lmx.limanix.dev/v1";

/// `PATH` of the system tasks: the tools of the running system, as the host's login shell had them.
const SYSTEM_PATH: &str = "/run/current-system/sw/bin";

/// Shell script that lists garbage-collector roots, leaving out those that always exist: running
/// processes, runtime state and the system profile. `$1` is `nix-store` and `$2` is `grep`. Its exit
/// status is ignored, as the platform's store guard ignored it.
const ROOTS_SCRIPT: &str = r#""$1" --gc --print-roots | "$2" -E -v -e '^"?/proc/' -e '^"?/run/' -e '^"?/nix/var/nix/profiles/system' -e '[{]censored[}]'
exit 0"#;

/// Shell script of the health check: every platform unit is active, and the development account
/// runs a command in its login shell, as the host checked a new generation. `$1` is `systemctl`,
/// `$2` `sudo`, `$3` `bash` and `$4` the account; the units follow. The last line names a failure.
const HEALTH_SCRIPT: &str = r#"systemctl=$1 sudo=$2 bash=$3 user=$4
shift 4
for unit in "$@"; do
  "$systemctl" is-active --quiet -- "$unit" || { echo "$unit is not active"; exit 1; }
done
"$sudo" --set-home --user "$user" -- "$bash" --login -c 'cd -- "$HOME" && exec "$@"' limanix-command true \
  || { echo "$user cannot run a command"; exit 1; }"#;

/// Shell script of finalize: remove the older generations of the system profile, then rewrite the
/// boot entries so they offer only what is kept. `$1` is `nix-env`, `$2` the profile and `$3`
/// `systemd-run`.
///
/// `switch-to-configuration` runs in its own unit, as `nixos-rebuild` runs it: stopping `lmxd` or
/// cancelling the task never interrupts a boot loader update, and the shared unit name keeps it from
/// running beside one that `nixos-rebuild` started.
const FINALIZE_SCRIPT: &str = r#""$1" --profile "$2" --delete-generations old &&
"$3" --collect --no-ask-password --pipe --quiet --service-type=exec \
  --unit=nixos-rebuild-switch-to-configuration --wait "$2/bin/switch-to-configuration" boot"#;

/// Workload kinds of `lmxd`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Collects unreferenced store paths with `nix-store --gc`.
    StoreCollect,
    /// Prints the garbage-collector roots that keep store paths alive.
    StoreRoots,
    /// Builds the mounted generation for the next boot with `nixos-rebuild boot`.
    SystemApply,
    /// Checks that the booted generation works.
    SystemHealth,
    /// Removes the older generations of the system profile and rewrites the boot entries.
    SystemFinalize,
}

impl Kind {
    /// Every kind, in declaration order.
    const ALL: [Self; 5] = [
        Self::StoreCollect,
        Self::StoreRoots,
        Self::SystemApply,
        Self::SystemHealth,
        Self::SystemFinalize,
    ];

    /// Kind name under [`API_VERSION`].
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::StoreCollect => "StoreCollect",
            Self::StoreRoots => "StoreRoots",
            Self::SystemApply => "SystemApply",
            Self::SystemHealth => "SystemHealth",
            Self::SystemFinalize => "SystemFinalize",
        }
    }

    /// Prefix of the names of this kind's tasks; a counter follows it.
    pub(crate) const fn task_prefix(self) -> &'static str {
        match self {
            Self::StoreCollect => "store-collect",
            Self::StoreRoots => "store-roots",
            Self::SystemApply => "system-apply",
            Self::SystemHealth => "system-health",
            Self::SystemFinalize => "system-finalize",
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

/// Workload of a `SystemApply` task for `generation`.
pub(crate) fn apply(generation: &str) -> ModelResult<TaskWorkload> {
    extension(Kind::SystemApply, json!({"generation": generation}))
}

/// Workload of `kind` with `spec`.
fn extension(kind: Kind, spec: Value) -> ModelResult<TaskWorkload> {
    ExtensionWorkload::new(API_VERSION, kind.name(), spec).map(TaskWorkload::Extension)
}

/// What the runner needs to turn a task into a process.
#[derive(Clone, Debug)]
pub(crate) struct Setup {
    /// Absolute paths of the programs the kinds run.
    pub(crate) tools: Tools,
    /// Guest locations.
    pub(crate) paths: Paths,
    /// Development account, which the health check runs a command as.
    pub(crate) user: String,
    /// Platform units the health check requires.
    pub(crate) units: Vec<String>,
}

/// A program to run, with its arguments and environment.
#[derive(Debug, PartialEq, Eq)]
struct Process {
    /// Absolute path of the program.
    program: String,
    /// Its arguments.
    args: Vec<String>,
    /// Its environment; the subprocess runner clears everything else.
    env: Vec<(String, String)>,
}

impl Process {
    /// `program` with `args` and an empty environment.
    fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            env: Vec::new(),
        }
    }
}

/// The process that runs a task of `kind` with `spec`.
fn process(kind: Kind, spec: &Value, setup: &Setup) -> Process {
    let tools = &setup.tools;
    match kind {
        Kind::StoreCollect => {
            let collect = [tools.nix_store.clone(), "--gc".into(), "--quiet".into()];
            if spec.get("priority").and_then(Value::as_str) == Some(Priority::Idle.as_str()) {
                let mut args = vec!["-n".into(), "19".into(), tools.ionice.clone()];
                args.extend(["-c".into(), "3".into()]);
                args.extend(collect);
                Process::new(tools.nice.clone(), args)
            } else {
                let [program, args @ ..] = collect;
                Process::new(program, args.to_vec())
            }
        }
        Kind::StoreRoots => Process::new(
            "/bin/sh",
            vec![
                "-c".into(),
                ROOTS_SCRIPT.into(),
                "sh".into(),
                tools.nix_store.clone(),
                tools.grep.clone(),
            ],
        ),
        Kind::SystemApply => {
            let mut process = Process::new(
                tools.nixos_rebuild.clone(),
                vec![
                    "boot".into(),
                    "--flake".into(),
                    setup.paths.flake(),
                    "--no-write-lock-file".into(),
                    "--no-update-lock-file".into(),
                ],
            );
            // The user's variables come last, so they win, as with systemd's `EnvironmentFile`.
            process.env.push(("PATH".into(), SYSTEM_PATH.into()));
            if let Ok(text) = fs::read_to_string(setup.paths.environment().join("environment")) {
                process.env.extend(environment::variables(&text));
            }
            process
        }
        Kind::SystemHealth => {
            let mut args = vec![
                "-c".into(),
                HEALTH_SCRIPT.into(),
                "sh".into(),
                tools.systemctl.clone(),
                tools.sudo.clone(),
                tools.bash.clone(),
                setup.user.clone(),
            ];
            args.extend(setup.units.iter().cloned());
            system(Process::new("/bin/sh", args))
        }
        Kind::SystemFinalize => system(Process::new(
            "/bin/sh",
            vec![
                "-c".into(),
                FINALIZE_SCRIPT.into(),
                "sh".into(),
                tools.nix_env.clone(),
                setup.paths.system_profile().display().to_string(),
                tools.systemd_run.clone(),
            ],
        )),
    }
}

/// `process` with the system tools in its `PATH`, which `switch-to-configuration` and the account's
/// login need.
fn system(mut process: Process) -> Process {
    process.env.push(("PATH".into(), SYSTEM_PATH.into()));
    process
}

/// Runner of the `lmxd` kinds.
struct LmxRunner {
    /// Catalog of the private subprocess runner.
    catalog: RunnerCatalog,
    /// What turns a task into a process.
    setup: Setup,
    /// Listeners of task output.
    capture: Arc<Capture>,
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
        let Process { program, args, env } = process(kind, workload.spec(), &self.setup);
        let mut task_env = TaskEnv::new();
        for (name, value) in env {
            task_env.push(name, value);
        }
        let subprocess = TaskWorkload::Subprocess(SubprocessSpec::new(
            SubprocessMode::Command {
                command: program,
                args,
            },
            task_env,
            None,
            Flag::enabled(),
        ));
        // Same name, generation and status, so output and runs belong to the `lmxd` task.
        let derived = Task::from_parts(
            task.type_meta().clone(),
            task.metadata().clone(),
            task.spec()
                .derive_with_workload(subprocess)
                .without_runner_selector(),
            task.status().clone(),
        )
        .map_err(|error| RunnerError::InvalidSpec(error.to_string()))?;
        let tee = Arc::new(Tee {
            inner: Arc::clone(ctx.output_publisher()),
            capture: Arc::clone(&self.capture),
        });
        let built = self
            .catalog
            .build_scoped_with_cancellation(
                &derived,
                &ctx.clone().with_output_publisher(tee),
                cancellation,
                scope,
            )
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
    setup: Setup,
    capture: Arc<Capture>,
) -> Result<Arc<SubprocessRunner>, RegisterError> {
    let mut private = RunnerRouter::new();
    let subprocess = register_subprocess_runner(&mut private, "lmx-exec")?;
    router.register(Arc::new(LmxRunner {
        catalog: private.catalog(),
        setup,
        capture,
    }))?;
    Ok(subprocess)
}

#[cfg(test)]
mod tests {
    use std::{os::unix::fs::PermissionsExt, path::Path, process::Command};

    use super::*;

    /// A setup with recognizable tool paths below `root`.
    fn setup(root: &Path) -> Setup {
        Setup {
            tools: Tools {
                ip: "/bin/ip".into(),
                systemctl: "/bin/systemctl".into(),
                nix_store: "/nix/bin/nix-store".into(),
                nice: "/bin/nice".into(),
                ionice: "/bin/ionice".into(),
                grep: "/bin/grep".into(),
                nixos_rebuild: "/bin/nixos-rebuild".into(),
                nix_env: "/bin/nix-env".into(),
                sudo: "/bin/sudo".into(),
                bash: "/bin/bash".into(),
                systemd_run: "/bin/systemd-run".into(),
            },
            paths: Paths::new(root.to_path_buf()),
            user: "dev".into(),
            units: vec!["sshd.service".into(), "lmx.socket".into()],
        }
    }

    #[test]
    fn collects_at_normal_priority() {
        let process = process(
            Kind::StoreCollect,
            &json!({"priority": "normal"}),
            &setup(Path::new("/")),
        );
        assert_eq!(process.program, "/nix/bin/nix-store");
        assert_eq!(process.args, ["--gc", "--quiet"]);
    }

    #[test]
    fn collects_for_the_guard_at_idle_priority() {
        let process = process(
            Kind::StoreCollect,
            &json!({"priority": "idle"}),
            &setup(Path::new("/")),
        );
        assert_eq!(process.program, "/bin/nice");
        assert_eq!(
            process.args,
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
        let process = process(Kind::StoreRoots, &json!({}), &setup(Path::new("/")));
        assert_eq!(process.program, "/bin/sh");
        assert_eq!(
            process.args[..2],
            ["-c".to_owned(), ROOTS_SCRIPT.to_owned()]
        );
        assert_eq!(process.args[2..], ["sh", "/nix/bin/nix-store", "/bin/grep"]);
    }

    #[test]
    fn builds_the_mounted_flake_with_the_users_environment() {
        let root = tempfile::tempdir().expect("temporary root");
        let setup = setup(root.path());
        fs::create_dir_all(setup.paths.environment()).expect("create /etc/limanix");
        fs::write(
            setup.paths.environment().join("environment"),
            "HTTP_PROXY=\"http://proxy:3128\"\nPATH=\"/opt/bin\"\n",
        )
        .expect("write the environment");

        let process = process(Kind::SystemApply, &json!({"generation": "g1"}), &setup);
        assert_eq!(process.program, "/bin/nixos-rebuild");
        let flake = format!("path:{}/mnt/limanix/flake#runtime", root.path().display());
        assert_eq!(
            process.args,
            [
                "boot",
                "--flake",
                flake.as_str(),
                "--no-write-lock-file",
                "--no-update-lock-file"
            ]
        );
        assert_eq!(
            process.env,
            [
                ("PATH".to_owned(), SYSTEM_PATH.to_owned()),
                ("HTTP_PROXY".to_owned(), "http://proxy:3128".to_owned()),
                ("PATH".to_owned(), "/opt/bin".to_owned()),
            ]
        );
    }

    #[test]
    fn checks_health_with_the_units_and_the_account() {
        let process = process(Kind::SystemHealth, &json!({}), &setup(Path::new("/")));
        assert_eq!(process.program, "/bin/sh");
        assert_eq!(
            process.args[2..],
            [
                "sh",
                "/bin/systemctl",
                "/bin/sudo",
                "/bin/bash",
                "dev",
                "sshd.service",
                "lmx.socket"
            ]
        );
    }

    #[test]
    fn finalizes_the_system_profile() {
        let process = process(Kind::SystemFinalize, &json!({}), &setup(Path::new("/")));
        assert_eq!(process.args[1], FINALIZE_SCRIPT);
        assert_eq!(
            process.args[2..],
            [
                "sh",
                "/bin/nix-env",
                "/nix/var/nix/profiles/system",
                "/bin/systemd-run"
            ]
        );
        assert_eq!(process.env, [("PATH".to_owned(), SYSTEM_PATH.to_owned())]);
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

**Step 11: Apply a generation**

Create `crates/lmxd/src/apply.rs`. One apply runs at a time; its followers share a broadcast channel and the outcome
under one lock, so a follower that joins never misses the outcome, and the first follower joins before the run starts.
`decide` is the request's decision as a pure function: a running apply of another generation is stopped before anything
is answered, even `restart_required`, because the host mounted another generation. A cancel marks the run, cancels its
task and waits for the outcome. The mounted generation is read again right before the build, which queues behind a
finalize in the `system` slot instead of killing it. After `stopping`, an apply that ends reports `owner.unavailable`:

```rust
//! The apply operation: building the mounted generation for the next boot.
//!
//! The host mounts a generation's inputs at `/mnt/limanix` and asks for it by name. An apply then
//! installs the generation's environment files, makes room in the store, and runs `nixos-rebuild
//! boot` as a `SystemApply` task. The apply belongs to `lmxd`: a caller that disconnects only stops
//! following it, and asking again attaches to the running one. Its state lives in memory; after a
//! reboot the generation markers tell what was built.

use std::{
    sync::{
        Arc, Mutex as SyncMutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use lmx_model::{
    Apply, ApplyEvent, ApplyPhase, ApplyState, BuildFailure, CancelApply, ErrorBody, ErrorCode,
    Generations, OutputStream,
};
use serde_json::{Map, Value};
use solti::{
    core::SupervisorApi,
    model::{AdmissionPolicy, TaskId, TaskPhase},
};
use tokio::sync::{Mutex, broadcast, watch};

use crate::{
    capture::{Capture, Line},
    environment,
    launch::{self, Ended, Placement},
    paths::Paths,
    store::Store,
    tasks::{self, Kind},
};

/// Slot of the system tasks.
pub(crate) const SYSTEM_SLOT: &str = "system";

/// Messages kept for a follower that falls behind; older ones are reported as lagged.
const BACKLOG: usize = 1024;

/// What `nix` prints, in lowercase, when the disk is full.
const DISK_FULL: &str = "no space left on device";

/// A message to the followers of an apply.
#[derive(Clone, Debug)]
pub(crate) enum Message {
    /// Progress.
    Event(ApplyEvent),
    /// How the apply ended; nothing follows.
    Outcome(Result<Apply, ErrorBody>),
}

/// How a caller joins an apply.
#[derive(Debug)]
pub(crate) enum Joined {
    /// The apply runs; its messages arrive until the outcome.
    Following(broadcast::Receiver<Message>),
    /// The apply has an outcome already.
    Done(Result<Apply, ErrorBody>),
}

/// Followers' channel and outcome of a run, under one lock so a new follower misses neither.
#[derive(Debug)]
struct Progress {
    /// Channel to the followers.
    messages: broadcast::Sender<Message>,
    /// The outcome, once there is one.
    outcome: Option<Result<Apply, ErrorBody>>,
}

/// One apply of a generation.
#[derive(Debug)]
pub(crate) struct Run {
    /// The generation being applied.
    generation: String,
    /// Followers and outcome.
    progress: SyncMutex<Progress>,
    /// Becomes `true` when the apply is cancelled.
    cancelled: watch::Sender<bool>,
    /// The `SystemApply` task, once it exists.
    task: SyncMutex<Option<TaskId>>,
}

impl Run {
    /// A run of `generation` without followers.
    fn new(generation: &str) -> Self {
        Self {
            generation: generation.to_owned(),
            progress: SyncMutex::new(Progress {
                messages: broadcast::channel(BACKLOG).0,
                outcome: None,
            }),
            cancelled: watch::Sender::new(false),
            task: SyncMutex::new(None),
        }
    }

    /// The followers and outcome, locked.
    fn progress(&self) -> std::sync::MutexGuard<'_, Progress> {
        self.progress.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tells the followers about `event`.
    fn event(&self, event: ApplyEvent) {
        let _ = self.progress().messages.send(Message::Event(event));
    }

    /// Records the outcome and tells the followers.
    fn finish(&self, outcome: Result<Apply, ErrorBody>) {
        let mut progress = self.progress();
        progress.outcome = Some(outcome.clone());
        let _ = progress.messages.send(Message::Outcome(outcome));
    }

    /// Joins the run as a follower, or returns its outcome.
    fn join(&self) -> Joined {
        let progress = self.progress();
        match &progress.outcome {
            Some(outcome) => Joined::Done(outcome.clone()),
            None => Joined::Following(progress.messages.subscribe()),
        }
    }

    /// Whether the run has no outcome yet.
    fn running(&self) -> bool {
        self.progress().outcome.is_none()
    }

    /// Whether the run was cancelled.
    fn is_cancelled(&self) -> bool {
        *self.cancelled.borrow()
    }

    /// The `SystemApply` task, once it exists.
    fn task(&self) -> Option<TaskId> {
        self.task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// What an apply request does.
#[derive(Debug)]
enum Decision<'a> {
    /// The requested generation is not the mounted one.
    Mismatch,
    /// An apply of the requested generation runs: attach to it.
    Join(&'a Arc<Run>),
    /// An apply of another generation runs: stop it, then decide again.
    Replace(&'a Arc<Run>),
    /// The requested generation is built: only a restart is needed.
    Built,
    /// Start an apply.
    Start,
}

/// What a request for `requested` does, given the generation markers and the running apply.
///
/// A running apply of another generation is replaced even when the requested one is built: the host
/// mounted another generation, so the older build must not finish after the answer.
fn decide<'a>(
    requested: &str,
    generations: &Generations,
    running: Option<&'a Arc<Run>>,
) -> Decision<'a> {
    if generations.desired.as_deref() != Some(requested) {
        return Decision::Mismatch;
    }
    match running {
        Some(run) if run.generation == requested => Decision::Join(run),
        Some(run) => Decision::Replace(run),
        None if generations.built.as_deref() == Some(requested) => Decision::Built,
        None => Decision::Start,
    }
}

/// Applies generations, one at a time.
pub(crate) struct Applier {
    /// Supervisor of the build task.
    supervisor: Arc<SupervisorApi>,
    /// Store operations, for the reserve before a build.
    store: Arc<Store>,
    /// Copies of the build's output.
    capture: Arc<Capture>,
    /// Guest locations.
    paths: Paths,
    /// Group of the installed environment files.
    gid: u32,
    /// Number of the last build task, for unique task names.
    created: AtomicU64,
    /// The latest apply.
    current: Mutex<Option<Arc<Run>>>,
    /// Set when `lmxd` stops; an apply that ends from then on says so instead of how its task ended.
    stopping: AtomicBool,
}

impl Applier {
    /// An applier that runs its builds in `supervisor`.
    pub(crate) fn new(
        supervisor: Arc<SupervisorApi>,
        store: Arc<Store>,
        capture: Arc<Capture>,
        paths: Paths,
        gid: u32,
    ) -> Self {
        Self {
            supervisor,
            store,
            capture,
            paths,
            gid,
            created: AtomicU64::new(0),
            current: Mutex::new(None),
            stopping: AtomicBool::new(false),
        }
    }

    /// Starts the apply of `generation`, joins the running one, or answers at once.
    ///
    /// A generation other than the mounted one is a mismatch. A built one needs only a restart. An
    /// apply of another generation is cancelled first, and the decision made again once it stopped.
    pub(crate) async fn apply(self: &Arc<Self>, generation: &str) -> Result<Joined, ErrorBody> {
        let mut current = self.current.lock().await;
        loop {
            let (generations, _) = lmx_facts::generations::read(&self.paths.generations());
            let running = current.clone().filter(|run| run.running());
            match decide(generation, &generations, running.as_ref()) {
                Decision::Mismatch => {
                    return Err(mismatch(generation, generations.desired.as_deref()));
                }
                Decision::Join(run) => return Ok(run.join()),
                Decision::Replace(run) => self.stop(run).await,
                Decision::Built => return Ok(Joined::Done(Ok(restart_required(generation)))),
                Decision::Start => {
                    let run = Arc::new(Run::new(generation));
                    // Join before the run starts, so the follower sees its first phase.
                    let joined = run.join();
                    *current = Some(Arc::clone(&run));
                    tokio::spawn(Arc::clone(self).drive(run));
                    return Ok(joined);
                }
            }
        }
    }

    /// Cancels the running apply of `generation` and waits until it stops.
    pub(crate) async fn cancel(&self, generation: &str) -> CancelApply {
        let current = self.current.lock().await.clone();
        let Some(run) = current.filter(|run| run.generation == generation && run.running()) else {
            return CancelApply { cancelled: false };
        };
        self.stop(&run).await;
        CancelApply { cancelled: true }
    }

    /// Marks `lmxd` as stopping, before the supervisor cancels the tasks.
    pub(crate) fn stopping(&self) {
        self.stopping.store(true, Ordering::Relaxed);
    }

    /// Cancels `run`, and its build task once it exists, and waits until the run has an outcome.
    async fn stop(&self, run: &Run) {
        let joined = run.join();
        run.cancelled.send_replace(true);
        if let Some(task) = run.task() {
            let _ = self.supervisor.cancel_task(&task).await;
        }
        if let Joined::Following(mut messages) = joined {
            loop {
                match messages.recv().await {
                    Ok(Message::Outcome(_)) | Err(broadcast::error::RecvError::Closed) => break,
                    Ok(Message::Event(_)) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                }
            }
        }
    }

    /// Runs the steps of `run` and records its outcome.
    async fn drive(self: Arc<Self>, run: Arc<Run>) {
        // The steps run in their own task, so even a panic gives the followers an outcome.
        let steps = tokio::spawn({
            let applier = Arc::clone(&self);
            let run = Arc::clone(&run);
            async move { applier.steps(&run).await }
        });
        let mut outcome = steps.await.unwrap_or_else(|error| {
            Err(failure(
                ErrorCode::ApplyBuildFailed,
                format!("The apply stopped unexpectedly: {error}"),
            ))
        });
        // A stop cancels the build like `lmx apply cancel`, but the caller must hear that lmxd went.
        if outcome.is_err() && self.stopping.load(Ordering::Relaxed) {
            outcome = Err(failure(
                ErrorCode::OwnerUnavailable,
                "lmxd stopped before the apply ended; apply the generation again.".into(),
            ));
        }
        match &outcome {
            Ok(_) => tracing::info!(generation = run.generation, "built the generation"),
            Err(error) if error.code == ErrorCode::ApplyCancelled => {
                tracing::info!(generation = run.generation, "apply cancelled");
            }
            Err(error) => {
                tracing::warn!(
                    generation = run.generation,
                    error = error.message,
                    "apply failed"
                );
            }
        }
        run.finish(outcome);
    }

    /// Environment, reserve and build, stopping between them when cancelled.
    async fn steps(&self, run: &Run) -> Result<Apply, ErrorBody> {
        run.event(ApplyEvent::Phase {
            phase: ApplyPhase::Environment,
        });
        environment::install(&self.paths, self.gid).map_err(|error| {
            failure(
                ErrorCode::ApplyEnvironmentFailed,
                format!("The environment files cannot be installed: {error}"),
            )
        })?;
        stop_if_cancelled(run)?;

        run.event(ApplyEvent::Phase {
            phase: ApplyPhase::Reserve,
        });
        // A collection can take minutes; a cancel leaves it running for the store guard.
        let mut cancel = run.cancelled.subscribe();
        tokio::select! {
            reserved = self.store.reserve() => {
                if let Err(warning) = reserved {
                    run.event(ApplyEvent::Warning {
                        code: warning.code,
                        message: warning.message,
                    });
                }
            }
            _ = cancel.wait_for(|cancelled| *cancelled) => return Err(cancelled()),
        }

        run.event(ApplyEvent::Phase {
            phase: ApplyPhase::Build,
        });
        self.build(run).await
    }

    /// Runs `nixos-rebuild boot` and passes its output to the followers.
    async fn build(&self, run: &Run) -> Result<Apply, ErrorBody> {
        // The host may have mounted another generation meanwhile; never build it under this name.
        let (generations, _) = lmx_facts::generations::read(&self.paths.generations());
        if generations.desired.as_deref() != Some(run.generation.as_str()) {
            return Err(mismatch(&run.generation, generations.desired.as_deref()));
        }
        let number = self.created.fetch_add(1, Ordering::Relaxed) + 1;
        let name = format!("{}-{number}", Kind::SystemApply.task_prefix());
        let mut lines = self.capture.listen(&name);
        // Queued: a finalize in the slot finishes first. A build of another generation was
        // cancelled by the apply that replaced it.
        let placement = Placement {
            slot: SYSTEM_SLOT,
            admission: AdmissionPolicy::Queue,
            timeout: Duration::MAX,
        };
        let started = match tasks::apply(&run.generation) {
            Ok(workload) => launch::start(&self.supervisor, &name, workload, placement).await,
            Err(error) => Err(error.to_string()),
        };
        let task = started.map_err(|error| {
            self.capture.forget(&name);
            failure(
                ErrorCode::ApplyBuildFailed,
                format!("The build cannot start: {error}"),
            )
        })?;
        *run.task.lock().unwrap_or_else(PoisonError::into_inner) = Some(task.clone());
        // A cancel that came while the task was created did not see it.
        if run.is_cancelled() {
            let _ = self.supervisor.cancel_task(&task).await;
        }

        let mut full = false;
        let finished = launch::finished(&self.supervisor, &task);
        tokio::pin!(finished);
        let result = loop {
            tokio::select! {
                result = &mut finished => break result,
                Some(line) = lines.recv() => full |= forward(run, line),
            }
        };
        self.capture.forget(&name);
        while let Ok(line) = lines.try_recv() {
            full |= forward(run, line);
        }

        match result {
            Ok(()) => Ok(restart_required(&run.generation)),
            Err(ended) if run.is_cancelled() || ended.phase == TaskPhase::Canceled => {
                Err(cancelled())
            }
            Err(ended) => Err(build_failed(&ended, self.store.shortage(full))),
        }
    }
}

/// Passes a line of the build to the followers; returns whether it says the disk is full.
fn forward(run: &Run, line: Line) -> bool {
    let full = line.text.to_lowercase().contains(DISK_FULL);
    run.event(ApplyEvent::Output {
        stream: if line.stderr {
            OutputStream::Stderr
        } else {
            OutputStream::Stdout
        },
        line: line.text,
        truncated: line.truncated,
    });
    full
}

/// The answer for a generation that is built and needs a restart.
fn restart_required(generation: &str) -> Apply {
    Apply {
        generation: generation.to_owned(),
        state: ApplyState::RestartRequired,
    }
}

/// A failure with `code` and `message`, without details.
fn failure(code: ErrorCode, message: String) -> ErrorBody {
    ErrorBody {
        code,
        message,
        details: Map::new(),
    }
}

/// The failure of an apply for a generation that is not the mounted one.
fn mismatch(requested: &str, mounted: Option<&str>) -> ErrorBody {
    let mut error = failure(
        ErrorCode::GenerationMismatch,
        format!(
            "Generation {requested} is not the one mounted at /mnt/limanix ({}).",
            mounted.unwrap_or("none")
        ),
    );
    error.details.insert("requested".into(), requested.into());
    error
        .details
        .insert("mounted".into(), mounted.map_or(Value::Null, Value::from));
    error
}

/// The failure of a cancelled apply.
fn cancelled() -> ErrorBody {
    failure(ErrorCode::ApplyCancelled, "The apply was cancelled.".into())
}

/// Stops the apply when it was cancelled.
fn stop_if_cancelled(run: &Run) -> Result<(), ErrorBody> {
    if run.is_cancelled() {
        Err(cancelled())
    } else {
        Ok(())
    }
}

/// The failure of a build that ended as `ended`; `disk` is the usage of a full disk.
fn build_failed(ended: &Ended, disk: Option<lmx_model::DiskUsage>) -> ErrorBody {
    let message = ended.exit_code.map_or_else(
        || format!("nixos-rebuild failed: {}", ended.message),
        |code| format!("nixos-rebuild failed with exit status {code}."),
    );
    let details = BuildFailure {
        exit_code: ended.exit_code,
        disk,
    };
    let mut error = failure(ErrorCode::ApplyBuildFailed, message);
    if let Ok(Value::Object(details)) = serde_json::to_value(details) {
        error.details = details;
    }
    error
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn decides_between_mismatch_join_replace_built_and_start() {
        let generations = |desired: &str, built: &str| Generations {
            desired: Some(desired.to_owned()),
            built: Some(built.to_owned()),
            booted: Some("g0".to_owned()),
        };
        let g1 = Arc::new(Run::new("g1"));
        let g2 = Arc::new(Run::new("g2"));
        let mounted_g2 = generations("g2", "g1");
        assert!(matches!(
            decide("g3", &mounted_g2, None),
            Decision::Mismatch
        ));
        assert!(matches!(
            decide("g2", &mounted_g2, Some(&g2)),
            Decision::Join(_)
        ));
        assert!(matches!(
            decide("g2", &mounted_g2, Some(&g1)),
            Decision::Replace(_)
        ));
        assert!(matches!(decide("g2", &mounted_g2, None), Decision::Start));
        assert!(matches!(
            decide("g2", &generations("g2", "g2"), None),
            Decision::Built
        ));
        assert!(
            matches!(
                decide("g1", &generations("g1", "g1"), Some(&g2)),
                Decision::Replace(_)
            ),
            "a build of another generation never outlives the answer"
        );
    }

    #[test]
    fn a_mismatch_names_both_generations() {
        let error = mismatch("g2", Some("g1"));
        assert_eq!(error.code, ErrorCode::GenerationMismatch);
        assert_eq!(
            Value::Object(error.details),
            json!({"requested": "g2", "mounted": "g1"})
        );
        assert_eq!(
            Value::Object(mismatch("g2", None).details)["mounted"],
            Value::Null
        );
    }

    #[test]
    fn a_failed_build_reports_its_exit_status() {
        let ended = Ended {
            phase: TaskPhase::Exhausted,
            exit_code: Some(1),
            message: "process exited with non-zero code: 1".into(),
        };
        let error = build_failed(&ended, None);
        assert_eq!(error.message, "nixos-rebuild failed with exit status 1.");
        assert_eq!(Value::Object(error.details), json!({"exit_code": 1}));
    }

    #[tokio::test]
    async fn a_follower_that_joins_first_sees_every_message() {
        let run = Run::new("g1");
        let Joined::Following(mut messages) = run.join() else {
            panic!("a new run has no outcome");
        };
        run.event(ApplyEvent::Phase {
            phase: ApplyPhase::Environment,
        });
        run.finish(Ok(restart_required("g1")));

        assert!(matches!(
            messages.recv().await,
            Ok(Message::Event(ApplyEvent::Phase { .. }))
        ));
        assert!(matches!(messages.recv().await, Ok(Message::Outcome(Ok(_)))));
        assert!(matches!(run.join(), Joined::Done(Ok(_))));
    }
}
```

**Step 12: Serve `Apply` and `CancelApply`**

Replace `crates/lmxd/src/owner.rs`. Without `follow`, or when the answer is known at once, the stream carries one
outcome event. A follower gets a forwarder task that passes the run's messages on and turns a lag into a `lagged` event;
a follower that disconnects only stops its forwarder:

```rust
//! The `lmx.v1.Owner` service.

use std::{sync::Arc, time::UNIX_EPOCH};

use lmx_ipc::{
    outcome_event,
    proto::{
        self, ApplyRequest, CancelApplyRequest, CancelApplyResponse, ReserveRequest,
        ReserveResponse, StatusRequest, StatusResponse, cancel_apply_response,
        reserve_response::Outcome,
    },
};
use lmx_model::{Apply, ApplyEvent, ApplyState, ErrorBody, ErrorCode, Operation, Owner};
use serde_json::Map;
use solti::{
    api::ApiIdentity,
    core::SupervisorApi,
    model::{TaskQuery, TaskWorkload},
};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::{
    apply::{Applier, Joined, Message},
    auth, launch,
    store::Store,
    tasks::API_VERSION,
};

/// Events buffered for a follower whose connection is slow.
const FOLLOW_BUFFER: usize = 64;

/// Handler of `lmx.v1.Owner`.
pub(crate) struct OwnerService {
    /// Store operations.
    pub(crate) store: Arc<Store>,
    /// The apply operation.
    pub(crate) applier: Arc<Applier>,
    /// Supervisor whose tasks are the operations.
    pub(crate) supervisor: Arc<SupervisorApi>,
    /// User `lmxd` runs as.
    pub(crate) own_uid: u32,
}

#[tonic::async_trait]
impl proto::owner_server::Owner for OwnerService {
    type ApplyStream = ReceiverStream<Result<proto::ApplyEvent, Status>>;

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
            Outcome::Failure(denied("Only root may reserve room in the store.").into())
        };
        Ok(Response::new(ReserveResponse {
            outcome: Some(outcome),
        }))
    }

    async fn apply(
        &self,
        request: Request<ApplyRequest>,
    ) -> Result<Response<Self::ApplyStream>, Status> {
        let privileged = auth::privileged(request.extensions().get(), self.own_uid);
        let ApplyRequest { generation, follow } = request.into_inner();
        let (events, stream) = mpsc::channel(FOLLOW_BUFFER);
        let outcome = if privileged {
            match self.applier.apply(&generation).await {
                Ok(Joined::Following(messages)) if follow => {
                    tokio::spawn(forward(messages, events));
                    return Ok(Response::new(ReceiverStream::new(stream)));
                }
                Ok(Joined::Following(_)) => Ok(Apply {
                    generation,
                    state: ApplyState::Running,
                }),
                Ok(Joined::Done(outcome)) => outcome,
                Err(error) => Err(error),
            }
        } else {
            Err(denied("Only root may apply a generation."))
        };
        // The stream is new and its buffer empty, so the one event fits.
        let _ = events.try_send(Ok(outcome_event(outcome)));
        Ok(Response::new(ReceiverStream::new(stream)))
    }

    async fn cancel_apply(
        &self,
        request: Request<CancelApplyRequest>,
    ) -> Result<Response<CancelApplyResponse>, Status> {
        let privileged = auth::privileged(request.extensions().get(), self.own_uid);
        let outcome = if privileged {
            let cancelled = self.applier.cancel(&request.into_inner().generation).await;
            cancel_apply_response::Outcome::Cancelled(cancelled.cancelled)
        } else {
            cancel_apply_response::Outcome::Failure(denied("Only root may cancel an apply.").into())
        };
        Ok(Response::new(CancelApplyResponse {
            outcome: Some(outcome),
        }))
    }
}

/// Passes the messages of an apply to a follower until the outcome, or until it disconnects.
async fn forward(
    mut messages: broadcast::Receiver<Message>,
    events: mpsc::Sender<Result<proto::ApplyEvent, Status>>,
) {
    loop {
        let event = match messages.recv().await {
            Ok(Message::Event(event)) => event.into(),
            Ok(Message::Outcome(outcome)) => {
                let _ = events.send(Ok(outcome_event(outcome))).await;
                return;
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                ApplyEvent::Lagged { skipped }.into()
            }
            Err(broadcast::error::RecvError::Closed) => return,
        };
        if events.send(Ok(event)).await.is_err() {
            return;
        }
    }
}

/// The failure of a caller who may not change the system.
fn denied(message: &str) -> ErrorBody {
    ErrorBody {
        code: ErrorCode::PermissionDenied,
        message: message.into(),
        details: Map::new(),
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
            (workload.api_version() == API_VERSION && launch::runs(task.status())).then(|| {
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

**Step 13: Start the applier with the daemon**

In `crates/lmxd/src/daemon.rs`, the imports:

```rust
use std::{future::Future, path::Path, sync::Arc, time::Duration};
```

with:

```rust
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
```

and

```rust
use crate::{
    auth::{PeerIdentity, TaskAccess},
    journal::Journal,
    owner::OwnerService,
    store::{self, GuardSchedule, Store, UsageSource},
    tasks::{self, RegisterError},
};
```

with:

```rust
use crate::{
    apply::Applier,
    auth::{PeerIdentity, TaskAccess},
    capture::Capture,
    journal::Journal,
    owner::OwnerService,
    paths::Paths,
    store::{self, GuardSchedule, Store, UsageSource},
    tasks::{self, RegisterError, Setup},
};
```

the system root in the options:

```rust
    /// When the store guard checks the disk; `None` turns the guard off.
    pub guard: Option<GuardSchedule>,
}
```

with:

```rust
    /// When the store guard checks the disk; `None` turns the guard off.
    pub guard: Option<GuardSchedule>,
    /// System root of the guest paths besides the store and the socket: `/` in a guest, a prepared
    /// tree in tests.
    pub root: PathBuf,
}
```

and

```rust
            .field("guard", &self.guard)
            .finish_non_exhaustive()
```

with:

```rust
            .field("guard", &self.guard)
            .field("root", &self.root)
            .finish_non_exhaustive()
```

the applier in the daemon:

```rust
    /// Store operations.
    store: Arc<Store>,
    /// The store guard, when it runs.
```

with:

```rust
    /// Store operations.
    store: Arc<Store>,
    /// The apply operation.
    applier: Arc<Applier>,
    /// The store guard, when it runs.
```

and

```rust
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
```

with:

```rust
    pub async fn start(options: Options) -> Result<Self, Error> {
        let Options {
            config,
            usage,
            guard,
            root,
        } = options;
        check(&config)?;
        let paths = Paths::new(root);
        let capture = Arc::new(Capture::default());
        let setup = Setup {
            tools: config.tools.clone(),
            paths: paths.clone(),
            user: config.user.name.clone(),
            units: config.health.units.clone(),
        };
        let mut router = RunnerRouter::new();
        let subprocess = tasks::register(&mut router, setup, Arc::clone(&capture))?;
        let supervisor = Arc::new(
            SupervisorApi::builder(router)
                .with_output_sink(Arc::new(Journal))
                .start()
                .await?,
        );
        let store = Arc::new(Store::new(Arc::clone(&supervisor), config.disk, usage));
        let applier = Arc::new(Applier::new(
            Arc::clone(&supervisor),
            Arc::clone(&store),
            capture,
            paths,
            config.user.gid,
        ));
        let guard = guard.map(|schedule| tokio::spawn(store::guard(Arc::clone(&store), schedule)));
        Ok(Self {
            supervisor,
            subprocess,
            store,
            applier,
            guard,
        })
    }
```

the applier in the service:

```rust
        let owner = OwnerServer::new(OwnerService {
            store: Arc::clone(&self.store),
            supervisor: Arc::clone(&self.supervisor),
```

with:

```rust
        let owner = OwnerServer::new(OwnerService {
            store: Arc::clone(&self.store),
            applier: Arc::clone(&self.applier),
            supervisor: Arc::clone(&self.supervisor),
```

and the new tools in the configuration check:

```rust
        ("grep", &tools.grep),
    ] {
```

with:

```rust
        ("grep", &tools.grep),
        ("systemctl", &tools.systemctl),
        ("nixos_rebuild", &tools.nixos_rebuild),
        ("nix_env", &tools.nix_env),
        ("sudo", &tools.sudo),
        ("bash", &tools.bash),
        ("systemd_run", &tools.systemd_run),
    ] {
```

and, when stopping, the applier learns it before the supervisor cancels the build, so its followers hear that `lmxd`
stopped rather than that someone cancelled:

```rust
        if let Some(guard) = &self.guard {
            guard.abort();
        }
```

with:

```rust
        if let Some(guard) = &self.guard {
            guard.abort();
        }
        self.applier.stopping();
```

**Step 14: Add `--transient` and the system root to the binary**

In `crates/lmxd/src/main.rs`, the module documentation:

```rust
//! exists while the daemon restarts. Started without one, as the transient daemon of an update,
//! it binds the socket itself. It reports readiness and feeds the watchdog when systemd asks for
//! them, and stops in order on SIGTERM or SIGINT.
```

with:

```rust
//! exists while the daemon restarts. Started without one, it binds the socket itself. It reports
//! readiness and feeds the watchdog when systemd asks for them, and stops in order on SIGTERM or
//! SIGINT.
//!
//! The host starts a second, transient daemon from the mounted generation to update the system:
//! `lmxd --transient --config <package>/etc/lmx/config.json`. It applies, but does not run the store
//! guard, which belongs to the daemon of the booted system.
```

the arguments:

```rust
    #[arg(long, default_value = SOCKET_PATH)]
    socket: PathBuf,
}
```

with:

```rust
    #[arg(long, default_value = SOCKET_PATH)]
    socket: PathBuf,
    /// Run as the transient daemon of an update: without the store guard.
    #[arg(long)]
    transient: bool,
    /// System root of the guest paths besides the store and the socket; only tests change it.
    #[arg(long, default_value = "/", hide = true)]
    root: PathBuf,
}
```

and the options:

```rust
        guard: Some(GuardSchedule::after_boot(uptime())),
    })
```

with:

```rust
        guard: (!args.transient).then(|| GuardSchedule::after_boot(uptime())),
        root: args.root,
    })
```

A transient daemon binds the socket itself. Two daemons on one socket would apply at once, so a socket that a daemon
still serves is refused, not replaced:

```rust
    os::unix::fs::{FileTypeExt, PermissionsExt},
```

with:

```rust
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        net::UnixStream,
    },
```

and

```rust
    // A socket left by an earlier daemon is replaced; anything else at the path is not ours.
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path)?,
```

with:

```rust
    // A socket left by an earlier daemon is replaced. One that a daemon still serves, such as the
    // system daemon when the host starts a transient one, is refused: two daemons would apply at once.
    // Anything else at the path is not ours.
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            if UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("another lmxd serves {}", path.display()),
                ));
            }
            fs::remove_file(path)?;
        }
```

In `crates/lmxd/src/lib.rs`, the crate documentation:

```rust
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
```

with:

```rust
//! must not depend on a caller's session: room in the Nix store, and building a mounted generation
//! for the next boot.
//!
//! | Module        | Does                                                                   |
//! |---------------|------------------------------------------------------------------------|
//! | `store`       | the store guard, reserve, and the conditions of the store disk         |
//! | `apply`       | the apply of a mounted generation and its followers                    |
//! | `environment` | installing the environment files of a generation                       |
//! | `tasks`       | the `lmx.limanix.dev/v1` workload kinds and the runner that runs them  |
//! | `launch`      | starting tasks and waiting for them                                    |
//! | `capture`     | copies of task output for `lmxd` itself                                |
//! | `paths`       | guest locations below a system root                                    |
//! | `owner`       | the `lmx.v1.Owner` gRPC service                                        |
//! | `auth`        | caller identity from peer credentials, and who may do what             |
//! | `journal`     | task output in the daemon's log                                        |
//! | `daemon`      | startup, serving the socket, and the stopping order                    |
```

and the modules:

```rust
mod auth;
mod daemon;
mod journal;
mod owner;
mod store;
mod tasks;
```

with:

```rust
mod apply;
mod auth;
mod capture;
mod daemon;
mod environment;
mod journal;
mod launch;
mod owner;
mod paths;
mod store;
mod tasks;
```

**Step 15: Give the integration tests a system root**

In `crates/lmx/tests/owner.rs`, the daemon of the guest tree gets the tree as its root:

```rust
                usage,
                guard,
            }))
```

with:

```rust
                usage,
                guard,
                root: root.path().to_path_buf(),
            }))
```

**Step 16: Run the daemon tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmxd`

Expected: 22 tests pass, among them `decides_between_mismatch_join_replace_built_and_start`,
`a_follower_that_joins_first_sees_every_message`, `installs_the_files_for_root_and_the_group_only` and
`builds_the_mounted_flake_with_the_users_environment`.

---

## Task 4: Health check, finalize and the generation conditions

**Files:**
- Create: `crates/lmxd/src/observer.rs`
- Modify: `crates/lmxd/src/tasks.rs`, `crates/lmxd/src/paths.rs`, `crates/lmxd/src/store.rs`, `crates/lmxd/src/owner.rs`, `crates/lmxd/src/daemon.rs`, `crates/lmxd/src/main.rs`, `crates/lmxd/src/lib.rs`, `crates/lmx/tests/owner.rs`

After the restart into a new generation, the daemon of the booted system checks that the generation works and then
finalizes it, as the host's ready check and prune did. The observer looks 30 seconds after start and then every minute.
`Owner.Status` derives the generation conditions on every call from the markers, the system profile and the last health
check.

**Step 1: Start health checks and finalizes**

In `crates/lmxd/src/tasks.rs`, after the workload of `SystemApply`:



```rust
/// Workload of a `SystemHealth` task.
pub(crate) fn health() -> ModelResult<TaskWorkload> {
    extension(Kind::SystemHealth, json!({}))
}

/// Workload of a `SystemFinalize` task.
pub(crate) fn finalize() -> ModelResult<TaskWorkload> {
    extension(Kind::SystemFinalize, json!({}))
}
```

In `crates/lmxd/src/paths.rs`, the mount table, which the check reads in process:

```rust
use lmx_facts::generations::{GenerationPaths, PROFILES_PATH};
```

with:

```rust
use lmx_facts::{
    generations::{GenerationPaths, PROFILES_PATH},
    mounts::MOUNTINFO_PATH,
};
```



```rust
    /// Mount table of `lmxd`.
    pub(crate) fn mountinfo(&self) -> PathBuf {
        self.at(MOUNTINFO_PATH)
    }
```

After a finalize, the observer collects at idle priority. In `crates/lmxd/src/store.rs`:

```rust
    async fn collect(&self, priority: Priority) -> Result<(), String> {
```

with:

```rust
    pub(crate) async fn collect(&self, priority: Priority) -> Result<(), String> {
```

**Step 2: Watch the booted generation**

Create `crates/lmxd/src/observer.rs`. The decisions are pure functions with table tests: `conditions` derives the
generation conditions, and `step` decides whether a look checks, finalizes or rests, including the back-off after a
failed finalize, which runs again even when it removed the older generations before it failed. `Converged` waits for a
finished finalize. A failed check keeps the last line the task printed as its reason. After the check, which may take
minutes, the markers are read again, and the finalize is dropped rather than queued while a build holds the `system`
slot: after that build, the older generations would include the booted one:

```rust
//! The generation observer: health and finalize of a booted generation.
//!
//! After the host restarts the VM into a new generation, the system `lmxd` checks that the
//! generation works, then finalizes it: the older generations of the system profile are removed and
//! the boot entries rewritten, as the host's prune did after its ready check. A generation that fails
//! its check is `Degraded` and keeps the older generations for a rollback; it is checked again every
//! minute. The check judges an update; once the generation is finalized and healthy, it stops.

use std::{
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use lmx_model::{CONVERGED, Condition, DEGRADED, Generations, OUT_OF_DATE, RESTART_REQUIRED};
use solti::{
    core::SupervisorApi,
    model::{AdmissionPolicy, ModelResult, TaskWorkload},
};

use crate::{
    apply::SYSTEM_SLOT,
    capture::Capture,
    launch::{self, Placement},
    paths::Paths,
    store::Store,
    tasks::{self, Kind, Priority},
};

/// Slot of the health check, apart from the system slot so an apply never waits for it.
const HEALTH_SLOT: &str = "health";

/// Longest a health check may run; a check that hangs, such as on a stuck mount, fails.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// Longest a finalize may run.
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// When the observer looks at the generations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObserverSchedule {
    /// Wait before the first look, so the system can settle after boot.
    pub first: Duration,
    /// Wait after each look before the next one.
    pub every: Duration,
    /// Wait after a failed finalize before trying again.
    pub retry: Duration,
}

impl ObserverSchedule {
    /// The system daemon's schedule: 30 seconds after start, then every minute; a failed finalize
    /// is retried after 15 minutes.
    #[must_use]
    pub fn system() -> Self {
        Self {
            first: Duration::from_secs(30),
            every: Duration::from_secs(60),
            retry: Duration::from_secs(15 * 60),
        }
    }
}

/// Result of the last health check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Health {
    /// Not checked for the booted generation.
    Unknown,
    /// The last check passed.
    Healthy,
    /// The last check failed, for the reason given.
    Unhealthy(String),
}

/// Where the finalize of the booted generation stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Finalize {
    /// None runs, and none failed.
    Idle,
    /// A finalize runs; the boot entries may still list removed generations.
    Running,
    /// The last finalize failed; it may run again from the instant given.
    Failed(Instant),
}

/// Watches the booted generation.
pub(crate) struct Observer {
    /// Supervisor of the check and finalize tasks.
    supervisor: Arc<SupervisorApi>,
    /// Store operations, for the collection after finalize.
    store: Arc<Store>,
    /// Copies of the tasks' output, for failure reasons.
    capture: Arc<Capture>,
    /// Guest locations.
    paths: Paths,
    /// Home of the development account, which must be mounted.
    home: String,
    /// Result of the last health check.
    health: Mutex<Health>,
    /// Where the finalize stands.
    finalize: Mutex<Finalize>,
    /// Number of the last task, for unique task names.
    created: AtomicU64,
}

impl Observer {
    /// An observer that runs its tasks in `supervisor`.
    pub(crate) fn new(
        supervisor: Arc<SupervisorApi>,
        store: Arc<Store>,
        capture: Arc<Capture>,
        paths: Paths,
        home: String,
    ) -> Self {
        Self {
            supervisor,
            store,
            capture,
            paths,
            home,
            health: Mutex::new(Health::Unknown),
            finalize: Mutex::new(Finalize::Idle),
            created: AtomicU64::new(0),
        }
    }

    /// Result of the last health check.
    pub(crate) fn health(&self) -> Health {
        self.health
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Records the result of a health check.
    fn set_health(&self, health: Health) {
        *self.health.lock().unwrap_or_else(PoisonError::into_inner) = health;
    }

    /// Where the finalize stands.
    fn finalize(&self) -> Finalize {
        *self.finalize.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records where the finalize stands.
    fn set_finalize(&self, finalize: Finalize) {
        *self.finalize.lock().unwrap_or_else(PoisonError::into_inner) = finalize;
    }

    /// Whether a finalize runs or waits to be tried again, so the generation is not converged.
    pub(crate) fn finalizing(&self) -> bool {
        self.finalize() != Finalize::Idle
    }

    /// One look: check a settled generation that is not finalized, and finalize it when healthy.
    ///
    /// A failed finalize is tried again after `retry`.
    pub(crate) async fn tick(&self, retry: Duration) {
        let (generations, _) = lmx_facts::generations::read(&self.paths.generations());
        let Some(generation) = settled(&generations).map(ToOwned::to_owned) else {
            self.set_health(Health::Unknown);
            self.set_finalize(Finalize::Idle);
            return;
        };
        let kept = match lmx_facts::generations::system_generations(&self.paths.profiles()) {
            Ok(kept) => kept,
            Err(error) => {
                tracing::warn!(%error, "the system profile generations cannot be read");
                return;
            }
        };
        let Step::Check { finalize } = step(kept, &self.health(), self.finalize(), Instant::now())
        else {
            return;
        };

        let health = self.check().await;
        let healthy = health == Health::Healthy;
        self.set_health(health);
        if !(healthy && finalize) {
            return;
        }
        // The check takes a while: an apply may have built another generation meanwhile.
        let (generations, _) = lmx_facts::generations::read(&self.paths.generations());
        if settled(&generations) != Some(generation.as_str()) {
            return;
        }
        self.set_finalize(Finalize::Running);
        match self.run(Kind::SystemFinalize, tasks::finalize()).await {
            Ok(()) => {
                tracing::info!(generation, "finalized the booted generation");
                self.set_finalize(Finalize::Idle);
                if let Err(error) = self.store.collect(Priority::Idle).await {
                    tracing::warn!(%error, "Collecting unreferenced store paths failed.");
                }
            }
            Err(reason) => {
                tracing::warn!(
                    generation,
                    reason,
                    "finalizing the booted generation failed"
                );
                self.set_finalize(Finalize::Failed(Instant::now() + retry));
            }
        }
    }

    /// Checks the booted generation: the mounts in process, then the health task.
    async fn check(&self) -> Health {
        if let Err(reason) = self.mounts() {
            return Health::Unhealthy(reason);
        }
        match self.run(Kind::SystemHealth, tasks::health()).await {
            Ok(()) => Health::Healthy,
            Err(reason) => Health::Unhealthy(reason),
        }
    }

    /// Whether the generation inputs and the development account's home are mounted.
    fn mounts(&self) -> Result<(), String> {
        let mounts = lmx_facts::mounts::shared(&self.paths.mountinfo())
            .map_err(|error| error.to_string())?;
        for target in ["/mnt/limanix", self.home.as_str()] {
            if !mounts.iter().any(|mount| mount.target == target) {
                return Err(format!("{target} is not mounted"));
            }
        }
        Ok(())
    }

    /// Runs a task of `kind` once and waits; a failure gives the last line it printed, or why it
    /// ended.
    async fn run(&self, kind: Kind, workload: ModelResult<TaskWorkload>) -> Result<(), String> {
        let number = self.created.fetch_add(1, Ordering::Relaxed) + 1;
        let name = format!("{}-{number}", kind.task_prefix());
        let mut lines = self.capture.listen(&name);
        // A finalize never waits behind a build in the system slot: after the build, the older
        // generations would include the booted one. A dropped finalize is tried again later.
        let placement = match kind {
            Kind::SystemHealth => Placement {
                slot: HEALTH_SLOT,
                admission: AdmissionPolicy::Queue,
                timeout: HEALTH_TIMEOUT,
            },
            _ => Placement {
                slot: SYSTEM_SLOT,
                admission: AdmissionPolicy::DropIfRunning,
                timeout: FINALIZE_TIMEOUT,
            },
        };
        let started = match workload {
            Ok(workload) => launch::start(&self.supervisor, &name, workload, placement).await,
            Err(error) => Err(error.to_string()),
        };
        let result = match started {
            Ok(task) => launch::finished(&self.supervisor, &task).await,
            Err(error) => {
                self.capture.forget(&name);
                return Err(error);
            }
        };
        self.capture.forget(&name);
        let mut last = None;
        while let Ok(line) = lines.try_recv() {
            if !line.text.trim().is_empty() {
                last = Some(line.text);
            }
        }
        result.map_err(|ended| last.unwrap_or(ended.message))
    }
}

/// What one look does for a settled generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Nothing: the generation is finalized and healthy, or a finalize runs or waits for its retry.
    Rest,
    /// Check health; when healthy and `finalize`, finalize.
    Check {
        /// Whether older generations wait to be removed.
        finalize: bool,
    },
}

/// What to do at `now` for a settled generation with `kept` profile generations, the last `health`
/// and `finalize`.
///
/// A failed finalize runs again once its back-off ends, even when it removed the older generations
/// before it failed, so the boot entries are rewritten.
fn step(kept: usize, health: &Health, finalize: Finalize, now: Instant) -> Step {
    match finalize {
        Finalize::Running => Step::Rest,
        Finalize::Failed(at) if now < at => Step::Rest,
        Finalize::Failed(_) => Step::Check { finalize: true },
        Finalize::Idle if kept > 1 => Step::Check { finalize: true },
        Finalize::Idle if *health == Health::Healthy => Step::Rest,
        Finalize::Idle => Step::Check { finalize: false },
    }
}

/// The generation that is desired, built and booted, if all three agree.
fn settled(generations: &Generations) -> Option<&str> {
    let desired = generations.desired.as_deref()?;
    (generations.built.as_deref() == Some(desired)
        && generations.booted.as_deref() == Some(desired))
    .then_some(desired)
}

/// Conditions of the generations, given the system profile's generation count, the last health
/// check, and whether a finalize runs or waits to be tried again.
pub(crate) fn conditions(
    generations: &Generations,
    kept: Option<usize>,
    health: &Health,
    finalizing: bool,
) -> Vec<Condition> {
    let Some(desired) = generations.desired.as_deref() else {
        return Vec::new();
    };
    let condition = |kind: &str, message: String| Condition {
        kind: kind.to_owned(),
        message,
    };
    if generations.built.as_deref() != Some(desired) {
        return vec![condition(
            OUT_OF_DATE,
            format!("Generation {desired} is mounted but not built; apply it."),
        )];
    }
    if generations.booted.as_deref() != Some(desired) {
        return vec![condition(
            RESTART_REQUIRED,
            format!("Generation {desired} is built; restart the VM to boot it."),
        )];
    }
    match health {
        Health::Unhealthy(reason) => vec![condition(
            DEGRADED,
            format!("Generation {desired} is booted but unhealthy: {reason}"),
        )],
        Health::Healthy if kept == Some(1) && !finalizing => vec![condition(
            CONVERGED,
            format!("Generation {desired} is booted, healthy and finalized."),
        )],
        Health::Healthy | Health::Unknown => Vec::new(),
    }
}

/// Looks at the generations on `schedule` until the task is dropped.
pub(crate) async fn observe(observer: Arc<Observer>, schedule: ObserverSchedule) {
    tokio::time::sleep(schedule.first).await;
    loop {
        observer.tick(schedule.retry).await;
        tokio::time::sleep(schedule.every).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generations with the three stages given.
    fn generations(desired: &str, built: &str, booted: &str) -> Generations {
        let stage = |value: &str| (!value.is_empty()).then(|| value.to_owned());
        Generations {
            desired: stage(desired),
            built: stage(built),
            booted: stage(booted),
        }
    }

    /// Kinds of the conditions for `generations`, `kept` and `health`, without a finalize.
    fn kinds(generations: &Generations, kept: Option<usize>, health: &Health) -> Vec<String> {
        conditions(generations, kept, health, false)
            .into_iter()
            .map(|condition| condition.kind)
            .collect()
    }

    #[test]
    fn follows_a_generation_from_mount_to_convergence() {
        let healthy = Health::Healthy;
        assert_eq!(
            kinds(&generations("g2", "g1", "g1"), Some(1), &healthy),
            [OUT_OF_DATE]
        );
        assert_eq!(
            kinds(&generations("g2", "g2", "g1"), Some(2), &healthy),
            [RESTART_REQUIRED]
        );
        assert!(kinds(&generations("g2", "g2", "g2"), Some(2), &healthy).is_empty());
        assert_eq!(
            kinds(&generations("g2", "g2", "g2"), Some(1), &healthy),
            [CONVERGED]
        );
    }

    #[test]
    fn an_unhealthy_booted_generation_is_degraded() {
        let health = Health::Unhealthy("sshd.service is not active".into());
        let conditions = conditions(&generations("g2", "g2", "g2"), Some(2), &health, false);
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].kind, DEGRADED);
        assert!(
            conditions[0]
                .message
                .ends_with("sshd.service is not active")
        );
    }

    #[test]
    fn finalizes_when_older_generations_are_kept_and_backs_off_after_a_failure() {
        let now = Instant::now();
        let later = Finalize::Failed(now + Duration::from_secs(60));
        let due = Finalize::Failed(now);
        let healthy = Health::Healthy;
        let unhealthy = Health::Unhealthy("sshd.service is not active".into());
        let check = |finalize| Step::Check { finalize };
        assert_eq!(step(2, &Health::Unknown, Finalize::Idle, now), check(true));
        assert_eq!(step(2, &unhealthy, Finalize::Idle, now), check(true));
        assert_eq!(
            step(2, &healthy, later, now),
            Step::Rest,
            "waits for the retry"
        );
        assert_eq!(
            step(1, &healthy, due, now),
            check(true),
            "rewrites the boot entries"
        );
        assert_eq!(step(1, &healthy, Finalize::Running, now), Step::Rest);
        assert_eq!(step(1, &Health::Unknown, Finalize::Idle, now), check(false));
        assert_eq!(step(1, &unhealthy, Finalize::Idle, now), check(false));
        assert_eq!(
            step(1, &healthy, Finalize::Idle, now),
            Step::Rest,
            "converged"
        );
    }

    #[test]
    fn says_nothing_without_markers_a_check_or_a_finished_finalize() {
        assert!(kinds(&generations("", "g1", "g1"), Some(1), &Health::Healthy).is_empty());
        assert!(kinds(&generations("g1", "g1", "g1"), Some(1), &Health::Unknown).is_empty());
        let settled = generations("g1", "g1", "g1");
        assert!(conditions(&settled, Some(1), &Health::Healthy, true).is_empty());
    }
}
```

**Step 3: Report the generation conditions**

In `crates/lmxd/src/owner.rs`, the imports:

```rust
use lmx_model::{Apply, ApplyEvent, ApplyState, ErrorBody, ErrorCode, Operation, Owner};
```

with:

```rust
use lmx_model::{Apply, ApplyEvent, ApplyState, Condition, ErrorBody, ErrorCode, Operation, Owner};
```

and

```rust
    auth, launch,
    store::Store,
```

with:

```rust
    auth, launch,
    observer::{self, Health, Observer},
    paths::Paths,
    store::Store,
```

the service's fields:

```rust
    pub(crate) applier: Arc<Applier>,
    /// Supervisor whose tasks are the operations.
```

with:

```rust
    pub(crate) applier: Arc<Applier>,
    /// The generation observer; `None` in a transient daemon.
    pub(crate) observer: Option<Arc<Observer>>,
    /// Guest locations.
    pub(crate) paths: Paths,
    /// Supervisor whose tasks are the operations.
```

the conditions, without a health check in a transient daemon:

```rust
impl OwnerService {
    /// Conditions of the generations and the store disk now.
    fn conditions(&self) -> Vec<Condition> {
        let (generations, _) = lmx_facts::generations::read(&self.paths.generations());
        let kept = lmx_facts::generations::system_generations(&self.paths.profiles()).ok();
        let (health, finalizing) = self
            .observer
            .as_ref()
            .map_or((Health::Unknown, false), |observer| {
                (observer.health(), observer.finalizing())
            });
        let mut conditions = observer::conditions(&generations, kept, &health, finalizing);
        conditions.extend(self.store.conditions());
        conditions
    }
}
```

and in `status`:

```rust
            conditions: self.store.conditions(),
```

with:

```rust
            conditions: self.conditions(),
```

**Step 4: Run the observer in the system daemon**

In `crates/lmxd/src/daemon.rs`, the imports:

```rust
    journal::Journal,
    owner::OwnerService,
```

with:

```rust
    journal::Journal,
    observer::{self, Observer, ObserverSchedule},
    owner::OwnerService,
```

the schedule in the options:

```rust
    pub guard: Option<GuardSchedule>,
    /// System root
```

with:

```rust
    pub guard: Option<GuardSchedule>,
    /// When the generation observer looks at the booted generation; `None` turns it off, as in the
    /// transient daemon of an update.
    pub observer: Option<ObserverSchedule>,
    /// System root
```

and

```rust
            .field("guard", &self.guard)
            .field("root", &self.root)
```

with:

```rust
            .field("guard", &self.guard)
            .field("observer", &self.observer)
            .field("root", &self.root)
```

the observer in the daemon, whose loop stops with the guard's:

```rust
    /// The apply operation.
    applier: Arc<Applier>,
    /// The store guard, when it runs.
    guard: Option<JoinHandle<()>>,
}
```

with:

```rust
    /// The apply operation.
    applier: Arc<Applier>,
    /// The generation observer, when it runs.
    observer: Option<Arc<Observer>>,
    /// Guest locations.
    paths: Paths,
    /// The store guard and the observer loop, while they run.
    background: Vec<JoinHandle<()>>,
}
```

and

```rust
            .debug_struct("Daemon")
            .field("guard", &self.guard.is_some())
```

with:

```rust
            .debug_struct("Daemon")
            .field("paths", &self.paths)
            .field("observer", &self.observer.is_some())
```

In `start`:

```rust
    /// Starts the supervisor, the runners and, when scheduled, the store guard.
    pub async fn start(options: Options) -> Result<Self, Error> {
        let Options {
            config,
            usage,
            guard,
            root,
        } = options;
```

with:

```rust
    /// Starts the supervisor, the runners and, when scheduled, the store guard and the generation
    /// observer.
    pub async fn start(options: Options) -> Result<Self, Error> {
        let Options {
            config,
            usage,
            guard,
            observer: observing,
            root,
        } = options;
```

and

```rust
            Arc::clone(&store),
            capture,
            paths,
            config.user.gid,
        ));
        let guard = guard.map(|schedule| tokio::spawn(store::guard(Arc::clone(&store), schedule)));
        Ok(Self {
            supervisor,
            subprocess,
            store,
            applier,
            guard,
        })
```

with:

```rust
            Arc::clone(&store),
            Arc::clone(&capture),
            paths.clone(),
            config.user.gid,
        ));

        let mut background = Vec::new();
        if let Some(schedule) = guard {
            background.push(tokio::spawn(store::guard(Arc::clone(&store), schedule)));
        }
        let observer = observing.map(|schedule| {
            let observer = Arc::new(Observer::new(
                Arc::clone(&supervisor),
                Arc::clone(&store),
                capture,
                paths.clone(),
                config.user.home.clone(),
            ));
            background.push(tokio::spawn(observer::observe(
                Arc::clone(&observer),
                schedule,
            )));
            observer
        });
        Ok(Self {
            supervisor,
            subprocess,
            store,
            applier,
            observer,
            paths,
            background,
        })
```

In `serve`, the service:

```rust
            applier: Arc::clone(&self.applier),
            supervisor: Arc::clone(&self.supervisor),
```

with:

```rust
            applier: Arc::clone(&self.applier),
            observer: self.observer.clone(),
            paths: self.paths.clone(),
            supervisor: Arc::clone(&self.supervisor),
```

and the stop:

```rust
        if let Some(guard) = &self.guard {
            guard.abort();
        }
```

with:

```rust
        for task in &self.background {
            task.abort();
        }
```

In `crates/lmxd/src/main.rs`, the module documentation:

```rust
//! `lmxd --transient --config <package>/etc/lmx/config.json`. It applies, but does not run the store
//! guard, which belongs to the daemon of the booted system.
```

with:

```rust
//! `lmxd --transient --config <package>/etc/lmx/config.json`. It applies, but runs neither the store
//! guard nor the generation observer, which belong to the daemon of the booted system.
```

the import:

```rust
use lmxd::{Daemon, GuardSchedule, Options};
```

with:

```rust
use lmxd::{Daemon, GuardSchedule, ObserverSchedule, Options};
```

the argument:

```rust
    /// Run as the transient daemon of an update: without the store guard.
```

with:

```rust
    /// Run as the transient daemon of an update: without the store guard and the generation
    /// observer.
```

and the options:

```rust
        guard: (!args.transient).then(|| GuardSchedule::after_boot(uptime())),
        root: args.root,
```

with:

```rust
        guard: (!args.transient).then(|| GuardSchedule::after_boot(uptime())),
        observer: (!args.transient).then(ObserverSchedule::system),
        root: args.root,
```

In `crates/lmxd/src/lib.rs`, the crate documentation:

```rust
//! must not depend on a caller's session: room in the Nix store, and building a mounted generation
//! for the next boot.
```

with:

```rust
//! must not depend on a caller's session: room in the Nix store, and updates of the system from
//! building a mounted generation to finalizing it after boot.
```

and

```rust
//! | `apply`       | the apply of a mounted generation and its followers                    |
```

with:

```rust
//! | `apply`       | the apply of a mounted generation and its followers                    |
//! | `observer`    | health check and finalize of the booted generation, and its conditions |
```

the module:

```rust
mod launch;
mod owner;
```

with:

```rust
mod launch;
mod observer;
mod owner;
```

and the schedule's export:

```rust
pub use daemon::{Daemon, Error, Options};
pub use store::{GuardSchedule, UsageSource};
```

with:

```rust
pub use daemon::{Daemon, Error, Options};
pub use observer::ObserverSchedule;
pub use store::{GuardSchedule, UsageSource};
```

The integration tests run without an observer until Task 5. In `crates/lmx/tests/owner.rs`:

```rust
                guard,
                root: root.path().to_path_buf(),
```

with:

```rust
                guard,
                observer: None,
                root: root.path().to_path_buf(),
```

**Step 5: Run the daemon tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmxd`

Expected: 26 tests pass, among them `follows_a_generation_from_mount_to_convergence` and
`finalizes_when_older_generations_are_kept_and_backs_off_after_a_failure`.

---

## Task 5: `lmx apply`, `lmx apply cancel` and `lmx status --wait converged`

**Files:**
- Create: `crates/lmx/src/apply.rs`, `crates/lmx/src/wait.rs`
- Modify: `crates/lmx/src/cli.rs`, `crates/lmx/src/main.rs`, `crates/lmx/src/owner.rs`, `crates/lmx/src/output.rs`, `crates/lmx/src/status.rs`, `crates/lmx/src/store.rs`
- Test: `crates/lmx/tests/owner.rs`, `crates/lmx/tests/cli.rs`

The host will run these commands over management SSH; people can run them too. The integration tests come first: they
start `lmxd` in the test process on a guest tree with fake tools and run the real `lmx` binary against it, as M2's tests
do.

**Step 1: Prepare the guest tree for updates**

In `crates/lmx/tests/owner.rs`, the module documentation:

```rust
//! `lmx` with a running `lmxd`: the owner part of `status`, and `store reserve`.
//!
//! Each test starts `lmxd` in this process on the socket of a prepared guest tree. The store disk is
//! read from a file, and a fake `nix-store` rewrites that file when it collects, so a test decides
//! what a collection frees.
```

with:

```rust
//! `lmx` with a running `lmxd`: the owner part of `status`, `store reserve`, `apply` and the wait for
//! a converged generation.
//!
//! Each test starts `lmxd` in this process on the socket of a prepared guest tree, which is also the
//! daemon's system root. The store disk is read from a file, and a fake `nix-store` rewrites that
//! file when it collects, so a test decides what a collection frees. Fake `nixos-rebuild`, `nix-env`,
//! `switch-to-configuration`, `systemctl` and `sudo` log their calls and change the tree as the real
//! ones change the system.
```

the imports:

```rust
    path::PathBuf,
```

with:

```rust
    path::{Path, PathBuf},
```

and

```rust
use lmxd::{Daemon, GuardSchedule, Options, UsageSource};
```

with:

```rust
use lmxd::{Daemon, GuardSchedule, ObserverSchedule, Options, UsageSource};
```

Before `priority_tool`, the fakes. Each logs its call into the tree's `calls`; the fake `nixos-rebuild` reads the file
`build` to fail or hang:

```rust
/// Fake `nixos-rebuild`. A build's `PATH` holds no tools in the tree, so it uses shell builtins and
/// absolute paths. It logs its call and prints a line on each stream; the stdout line shows a variable
/// of the user's environment. Then it does what the file `build` says: `fail` exits with status 1,
/// `hang` sleeps until it is killed, and anything else makes the mounted generation the built one.
const NIXOS_REBUILD: &str = r#"#!/bin/sh
root=${0%/bin/*}
printf 'nixos-rebuild %s\n' "$*" >> "$root/calls"
echo "building the system configuration..." >&2
echo "HTTP_PROXY=$HTTP_PROXY"
case $(/bin/cat "$root/build" 2>/dev/null) in
  fail) echo "error: builder for 'system.drv' failed" >&2; exit 1 ;;
  hang) exec /bin/sleep 60 ;;
esac
/bin/cp "$root/mnt/limanix/flake/runtime.json" "$root/nix/var/nix/profiles/system/etc/lmx/config.json"
"#;

/// Fake `nix-env`: logs its call; `--delete-generations old` removes the first generation link.
const NIX_ENV: &str = r#"#!/bin/sh
root=${0%/bin/*}
printf 'nix-env %s\n' "$*" >> "$root/calls"
/bin/rm -f "$2-1-link"
"#;

/// Fake `switch-to-configuration` inside the system profile: logs its call.
const SWITCH_TO_CONFIGURATION: &str = r#"#!/bin/sh
root=${0%/nix/var/nix/profiles/system/bin/*}
printf 'switch-to-configuration %s\n' "$*" >> "$root/calls"
"#;

/// Fake `systemctl`: logs its call; every unit is active.
const SYSTEMCTL: &str = r#"#!/bin/sh
root=${0%/bin/*}
printf 'systemctl %s\n' "$*" >> "$root/calls"
"#;

/// Fake `systemd-run`: logs its call, drops its options and runs the command in place.
const SYSTEMD_RUN: &str = r#"#!/bin/sh
root=${0%/bin/*}
printf 'systemd-run %s\n' "$*" >> "$root/calls"
while [ "${1#--}" != "$1" ]; do shift; done
exec "$@"
"#;

/// Fake `sudo`: logs the account it was asked to run a command as.
const SUDO: &str = r#"#!/bin/sh
root=${0%/bin/*}
printf 'sudo %s %s %s\n' "$1" "$2" "$3" >> "$root/calls"
"#;

/// Mount table with the generation inputs and the development account's home.
const MOUNTINFO: &str = "\
35 1 0:30 / /mnt/limanix ro,relatime - virtiofs mount0 ro
36 1 0:31 / /home/dev rw,relatime - virtiofs mount1 rw
";
```

A guest with an observer that looks every 200 milliseconds, and `start` with the schedule:

```rust
    /// A guest whose store disk is `before` until a collection makes it `after`.
    fn new(before: DiskUsage, after: DiskUsage) -> Self {
        Self::start(before, after, None, None)
    }

    /// A guest with plenty of room and a generation observer that looks every 200 milliseconds.
    fn observed() -> Self {
        let observer = ObserverSchedule {
            first: Duration::ZERO,
            every: Duration::from_millis(200),
            retry: Duration::from_secs(3600),
        };
        Self::start(usage(50), usage(50), None, Some(observer))
    }

    /// Like [`Guest::new`], with a store guard that checks the disk at once.
    fn with_guard(before: DiskUsage, after: DiskUsage) -> Self {
        let guard = GuardSchedule {
            first: Duration::ZERO,
            every: Duration::from_secs(3600),
        };
        Self::start(before, after, Some(guard), None)
    }

    /// Prepares the tree and starts `lmxd` with `guard` and `observer`.
    fn start(
        before: DiskUsage,
        after: DiskUsage,
        guard: Option<GuardSchedule>,
        observer: Option<ObserverSchedule>,
    ) -> Self {
```

The new fakes in the tool loop, written by a helper:

```rust
            ("ionice", priority_tool("ionice")),
        ] {
            let tool = path(&format!("bin/{tool}"));
            fs::write(&tool, script).expect("write a tool");
            fs::set_permissions(&tool, fs::Permissions::from_mode(0o755))
                .expect("make a tool executable");
        }
```

with:

```rust
            ("ionice", priority_tool("ionice")),
            ("nixos-rebuild", NIXOS_REBUILD.to_owned()),
            ("nix-env", NIX_ENV.to_owned()),
            ("systemctl", SYSTEMCTL.to_owned()),
            ("sudo", SUDO.to_owned()),
            ("systemd-run", SYSTEMD_RUN.to_owned()),
        ] {
            executable(&path(&format!("bin/{tool}")), &script);
        }
```

the schedule in the options:

```rust
                observer: None,
```

with:

```rust
                observer,
```

Methods that write a generation into the tree and wait for a call, before `calls`, whose documentation changes:

```rust
    /// Writes `content` to `relative` below the root.
    fn write(&self, relative: &str, content: &str) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().expect("file has a parent")).expect("create parent");
        fs::write(path, content).expect("write file");
    }

    /// Mounts generation `desired` with its environment files over a system that built `built` and
    /// booted `booted`, keeping `kept` generations in the system profile.
    fn mount(&self, desired: &str, built: &str, booted: &str, kept: usize) {
        let marker = |generation: &str| json!({"generation": generation}).to_string();
        self.write("mnt/limanix/flake/runtime.json", &marker(desired));
        self.write(
            "mnt/limanix/environment",
            "HTTP_PROXY=\"http://proxy:3128\"\n",
        );
        self.write(
            "mnt/limanix/environment.sh",
            "export HTTP_PROXY=\"http://proxy:3128\"\n",
        );
        self.write(
            "nix/var/nix/profiles/system/etc/lmx/config.json",
            &marker(built),
        );
        executable(
            &self.path("nix/var/nix/profiles/system/bin/switch-to-configuration"),
            SWITCH_TO_CONFIGURATION,
        );
        for number in 1..=kept {
            self.write(&format!("nix/var/nix/profiles/system-{number}-link"), "");
        }
        self.write("run/booted-system/etc/lmx/config.json", &marker(booted));
        self.write("proc/self/mountinfo", MOUNTINFO);
    }

    /// Waits up to 20 seconds until a logged call starts with `prefix`.
    fn wait_for_call(&self, prefix: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.calls().iter().any(|call| call.starts_with(prefix)) {
            assert!(
                Instant::now() < deadline,
                "no {prefix} in {:?}",
                self.calls()
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Every call the fake tools logged so far; `nix-store` logs only its arguments.
```

and the helpers before `answer`:

```rust
/// Writes an executable script at `path`.
fn executable(path: &Path, script: &str) {
    fs::create_dir_all(path.parent().expect("script has a parent")).expect("create parent");
    fs::write(path, script).expect("write a script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make it executable");
}

/// Parses standard output as JSON Lines: the events of `apply --follow`, then the envelope.
fn json_lines(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}")))
        .collect()
}
```

`lmx status` now runs the fake `systemctl`, which logs its call, so the M2 test looks for collections only:

```rust
    assert!(guest.calls().is_empty(), "status collects nothing");
```

with:

```rust
    assert!(
        !guest.calls().iter().any(|call| call.contains("--gc")),
        "status collects nothing"
    );
```

**Step 2: Write the failing tests of the update**

At the end of `crates/lmx/tests/owner.rs`, the four scenarios of the design: an apply succeeds, a build fails, a cancel
stops a build and its follower, and a healthy booted generation is finalized and converges:



```rust
#[test]
fn an_apply_installs_the_environment_and_builds_for_the_next_boot() {
    let guest = Guest::new(usage(50), usage(50));
    guest.mount("g2", "g1", "g1", 1);
    let output = guest
        .lmx(&["apply", "-g", "g2", "--follow", "--json"])
        .output()
        .expect("run lmx");
    assert!(output.status.success(), "{output:?}");

    let lines = json_lines(&output);
    let (envelope, events) = lines.split_last().expect("an answer");
    assert_eq!(
        envelope,
        &json!({"contract": 1, "ok": true, "data": {"generation": "g2", "state": "restart_required"}})
    );
    let phases: Vec<&Value> = events
        .iter()
        .filter(|event| event["event"] == "phase")
        .map(|event| &event["phase"])
        .collect();
    assert_eq!(phases, ["environment", "reserve", "build"]);
    for (stream, line) in [
        ("stderr", "building the system configuration..."),
        ("stdout", "HTTP_PROXY=http://proxy:3128"),
    ] {
        let event = json!({"event": "output", "stream": stream, "line": line});
        assert!(events.contains(&event), "{event} missing from {events:?}");
    }
    let environment = guest.path("etc/limanix/environment");
    assert_eq!(
        fs::read_to_string(&environment).expect("installed"),
        "HTTP_PROXY=\"http://proxy:3128\"\n"
    );
    assert_eq!(
        fs::metadata(&environment).expect("installed").mode() & 0o777,
        0o640
    );

    // Built now: asking again needs only a restart, and status says so.
    let again = guest
        .lmx(&["apply", "-g", "g2", "--json"])
        .output()
        .expect("run lmx");
    assert_eq!(answer(&again)["data"]["state"], "restart_required");
    let status = guest.lmx(&["status", "--json"]).output().expect("run lmx");
    assert_eq!(
        answer(&status)["data"]["owner"]["conditions"][0]["type"],
        "RestartRequired"
    );
    let other = guest
        .lmx(&["apply", "-g", "g3", "--json"])
        .output()
        .expect("run lmx");
    assert_eq!(other.status.code(), Some(1), "{other:?}");
    assert_eq!(
        answer(&other)["error"]["details"],
        json!({"requested": "g3", "mounted": "g2"})
    );
}

#[test]
fn a_failed_build_reports_its_exit_status() {
    let guest = Guest::new(usage(50), usage(50));
    guest.mount("g2", "g1", "g1", 1);
    guest.write("build", "fail");
    let output = guest
        .lmx(&["apply", "-g", "g2", "--follow", "--json"])
        .output()
        .expect("run lmx");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let lines = json_lines(&output);
    let envelope = lines.last().expect("an answer");
    assert_eq!(envelope["error"]["code"], "apply.build_failed");
    assert_eq!(envelope["error"]["details"], json!({"exit_code": 1}));
    assert!(lines.contains(&json!({
        "event": "output",
        "stream": "stderr",
        "line": "error: builder for 'system.drv' failed"
    })));
}

#[test]
fn a_cancel_stops_the_build_and_its_followers() {
    let guest = Guest::new(usage(50), usage(50));
    guest.mount("g2", "g1", "g1", 1);
    guest.write("build", "hang");
    let follower = guest
        .lmx(&["apply", "-g", "g2", "--follow", "--json"])
        .spawn()
        .expect("run lmx");
    guest.wait_for_call("nixos-rebuild");

    let cancel = guest
        .lmx(&["apply", "cancel", "-g", "g2", "--json"])
        .output()
        .expect("run lmx");
    assert!(cancel.status.success(), "{cancel:?}");
    assert_eq!(answer(&cancel)["data"], json!({"cancelled": true}));

    let output = follower.wait_with_output().expect("wait for lmx");
    assert_eq!(output.status.code(), Some(130), "{output:?}");
    let lines = json_lines(&output);
    assert_eq!(
        lines.last().expect("an answer")["error"]["code"],
        "apply.cancelled"
    );
    let again = guest
        .lmx(&["apply", "cancel", "-g", "g2", "--json"])
        .output()
        .expect("run lmx");
    assert_eq!(answer(&again)["data"], json!({"cancelled": false}));
}

#[test]
fn a_healthy_booted_generation_is_finalized_and_converges() {
    let guest = Guest::observed();
    guest.mount("g1", "g1", "g1", 2);
    let output = guest
        .lmx(&[
            "status",
            "--wait",
            "converged",
            "-g",
            "g1",
            "--timeout",
            "60s",
            "--json",
        ])
        .output()
        .expect("run lmx");
    assert!(output.status.success(), "{output:?}");

    let conditions = &answer(&output)["data"]["owner"]["conditions"];
    assert_eq!(conditions[0]["type"], "Converged", "{conditions}");
    assert!(!guest.path("nix/var/nix/profiles/system-1-link").exists());
    let profile = guest
        .path("nix/var/nix/profiles/system")
        .display()
        .to_string();
    let calls = guest.calls();
    for call in [
        "systemctl is-active --quiet -- sshd.service".to_owned(),
        "sudo --set-home --user dev".to_owned(),
        format!("nix-env --profile {profile} --delete-generations old"),
        format!(
            "systemd-run --collect --no-ask-password --pipe --quiet --service-type=exec \
             --unit=nixos-rebuild-switch-to-configuration --wait {profile}/bin/switch-to-configuration boot"
        ),
        "switch-to-configuration boot".to_owned(),
    ] {
        assert!(calls.contains(&call), "{call} missing from {calls:?}");
    }
}
```

In `crates/lmx/tests/cli.rs`, every owner operation reports an unavailable owner without `lmxd`. Replace
`store_reserve_without_lmxd_reports_an_unavailable_owner` with:

```rust
fn owner_operations_without_lmxd_report_an_unavailable_owner() {
    let guest = Guest::new();
    for args in [
        &["store", "reserve", "--json"][..],
        &["apply", "-g", "0123456789ab", "--json"],
        &["apply", "cancel", "-g", "0123456789ab", "--json"],
        &[
            "status",
            "--wait",
            "converged",
            "-g",
            "0123456789ab",
            "--timeout",
            "1s",
            "--json",
        ],
    ] {
        let output = guest.lmx(args, &guest.config());
        assert_eq!(output.status.code(), Some(3), "{args:?}: {output:?}");
        let answer: Value = serde_json::from_slice(&output.stdout).expect("one JSON answer");
        assert_eq!(answer["ok"], false, "{args:?}");
        assert_eq!(answer["error"]["code"], "owner.unavailable", "{args:?}");
    }
}
```

**Step 3: Run them to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx --test owner --test cli --no-fail-fast`

Expected: the new tests fail: `lmx` answers ``error: unrecognized subcommand 'apply'`` and ``error: unexpected argument
'--wait' found``; the M2 tests still pass.

**Step 4: Write JSON Lines and failures**

In `crates/lmx/src/output.rs`, an event is one JSON line like an envelope, and a failure is an envelope or a line on
standard error:

```rust
use lmx_model::Envelope;
```

with:

```rust
use lmx_model::{Envelope, ErrorBody};
```

and

```rust
/// Writes one envelope as a single JSON line to standard output.
pub(crate) fn write_json<T: Serialize>(envelope: &Envelope<T>) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, envelope)?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}
```

with:

```rust
/// Writes one envelope as a single JSON line to standard output.
pub(crate) fn write_json<T: Serialize>(envelope: &Envelope<T>) -> io::Result<()> {
    write_json_line(envelope)
}

/// Writes `value` as a single JSON line to standard output, such as one event of a stream.
pub(crate) fn write_json_line<T: Serialize>(value: &T) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}

/// Reports a failed command and exits with `status`: the JSON answer with `error`, or its message
/// on standard error.
pub(crate) fn failure(json: bool, error: ErrorBody, status: u8) -> io::Result<ExitCode> {
    if json {
        write_json(&Envelope::<()>::failure(error))?;
    } else {
        eprintln!("lmx: {}", error.message);
    }
    Ok(ExitCode::from(status))
}
```

**Step 5: Ask `lmxd` to apply and to cancel**

Replace `crates/lmx/src/owner.rs`. The probe that `lmx store reserve` runs before its call becomes `ready`, shared by
every operation. `apply` reads the stream and passes each event to the caller, whose write failures end the call as
`CallError::Output`. `lmxd` ends every stream with an outcome, so a stream that breaks or ends early means `lmxd` went
away. `report` turns a call without an answer into `owner.unavailable` with exit status 3, for every command:

```rust
//! Calls to the guest owner daemon `lmxd`.
//!
//! Owner operations run only in `lmxd`; `lmx` asks for them over the daemon's socket and never runs
//! them itself. A daemon that cannot be reached is reported, never replaced.

use std::{fmt, future::Future, io, path::Path, process::ExitCode, time::Duration};

use lmx_ipc::{
    ApplyMessage,
    proto::{
        ApplyRequest, CancelApplyRequest, ReserveRequest, StatusRequest, owner_client::OwnerClient,
    },
    tonic::{Code, Status, transport::Channel},
};
use lmx_model::{Apply, ApplyEvent, CancelApply, ErrorBody, ErrorCode, Owner, Reserve};
use serde_json::Map;

use crate::output;

/// How long `lmx status` waits for `lmxd` to connect and answer.
const STATUS_TIMEOUT: Duration = Duration::from_secs(2);

/// How long an owner operation waits for `lmxd` to answer at all before it asks for the operation.
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
    /// An answer could not be passed on, such as an event to a closed standard output.
    Output(io::Error),
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
            Self::Output(error) => write!(formatter, "{error}"),
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
pub(crate) fn reserve(socket: &Path) -> Result<Result<Reserve, ErrorBody>, CallError> {
    block_on(async {
        let mut client = ready(socket).await?;
        let response = client
            .reserve(ReserveRequest {})
            .await
            .map_err(|status| CallError::from_status(&status))?;
        lmx_ipc::reserve_outcome(response.into_inner())
            .map_err(|error| CallError::Failed(error.to_string()))
    })
}

/// Asks `lmxd` on `socket` to apply `generation`, passing each event to `event` until the outcome.
///
/// Without `follow`, `lmxd` answers at once that the apply runs or that the generation is built.
/// The apply belongs to `lmxd`: a caller that stops following leaves it running.
pub(crate) fn apply(
    socket: &Path,
    generation: &str,
    follow: bool,
    mut event: impl FnMut(ApplyEvent) -> io::Result<()>,
) -> Result<Result<Apply, ErrorBody>, CallError> {
    block_on(async {
        let mut client = ready(socket).await?;
        let request = ApplyRequest {
            generation: generation.to_owned(),
            follow,
        };
        let mut events = client
            .apply(request)
            .await
            .map_err(|status| CallError::from_status(&status))?
            .into_inner();
        // lmxd ends every stream with an outcome, so a stream that breaks or ends early lost lmxd.
        loop {
            let message = events
                .message()
                .await
                .map_err(|status| {
                    CallError::Unavailable(format!(
                        "lmxd stopped answering during the apply: {}",
                        status.message()
                    ))
                })?
                .ok_or_else(|| {
                    CallError::Unavailable("lmxd ended the apply without an outcome".into())
                })?;
            match lmx_ipc::apply_message(message)
                .map_err(|error| CallError::Failed(error.to_string()))?
            {
                ApplyMessage::Event(update) => event(update).map_err(CallError::Output)?,
                ApplyMessage::Outcome(outcome) => return Ok(outcome),
            }
        }
    })
}

/// Asks `lmxd` on `socket` to cancel the apply of `generation`; it answers once the apply stopped.
pub(crate) fn cancel_apply(
    socket: &Path,
    generation: &str,
) -> Result<Result<CancelApply, ErrorBody>, CallError> {
    block_on(async {
        let mut client = ready(socket).await?;
        let response = client
            .cancel_apply(CancelApplyRequest {
                generation: generation.to_owned(),
            })
            .await
            .map_err(|status| CallError::from_status(&status))?;
        lmx_ipc::cancel_outcome(response.into_inner())
            .map_err(|error| CallError::Failed(error.to_string()))
    })
}

/// Reports a call that gave no answer of the host contract.
///
/// An unreachable daemon is the contract's `owner.unavailable`, with exit status 3. A broken call
/// has no contract code and is reported on standard error only.
pub(crate) fn report(error: CallError, json: bool) -> io::Result<ExitCode> {
    match error {
        CallError::Unavailable(message) => output::failure(
            json,
            ErrorBody {
                code: ErrorCode::OwnerUnavailable,
                message,
                details: Map::new(),
            },
            output::UNAVAILABLE,
        ),
        CallError::Failed(message) => {
            eprintln!("lmx: {message}");
            Ok(ExitCode::from(output::FAILURE))
        }
        CallError::Output(error) => Err(error),
    }
}

/// Connects to `lmxd` on `socket` and checks that it answers within [`PROBE_TIMEOUT`], so a daemon
/// that never starts is reported as unavailable instead of waited for.
async fn ready(socket: &Path) -> Result<OwnerClient<Channel>, CallError> {
    let mut client = connect(socket).await?;
    tokio::time::timeout(PROBE_TIMEOUT, client.status(StatusRequest {}))
        .await
        .map_err(|_| silent(PROBE_TIMEOUT))?
        .map_err(|status| CallError::from_status(&status))?;
    Ok(client)
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

`lmx store reserve` reports through it. In `crates/lmx/src/store.rs`:

```rust
use lmx_model::{Envelope, ErrorBody, ErrorCode, Reserve, Shortage};
use serde_json::{Map, Value};

use crate::{
    cli::OutputArgs,
    format, output,
    owner::{self, CallError},
    system::System,
};
```

with:

```rust
use lmx_model::{Envelope, ErrorBody, Reserve, Shortage};
use serde_json::Value;

use crate::{cli::OutputArgs, format, output, owner, system::System};
```

and

```rust
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
```

with:

```rust
        Ok(Err(error)) => fail(args, error),
        Err(error) => owner::report(error, args.json),
    }
}

/// Reports a failed reserve; people also see the disk a shortage left.
fn fail(args: &OutputArgs, error: ErrorBody) -> io::Result<ExitCode> {
    let shortage = serde_json::from_value::<Shortage>(Value::Object(error.details.clone()));
    let status = output::failure(args.json, error, output::FAILURE)?;
    if let (false, Ok(shortage)) = (args.json, shortage) {
        eprintln!("Guest disk: {}.", format::disk(&shortage.after));
    }
    Ok(status)
}
```

**Step 6: Parse the new commands**

In `crates/lmx/src/cli.rs`, the imports:

```rust
use std::ffi::OsString;

use clap::{Arg, ArgAction, Args, Parser, Subcommand};
```

with:

```rust
use std::{ffi::OsString, time::Duration};

use clap::{Arg, ArgAction, Args, Parser, Subcommand, ValueEnum};
```

`lmx status` takes the wait:

```rust
    Status(OutputArgs),
```

with:

```rust
    Status(StatusArgs),
```

and `lmx apply` is a command:

```rust
    Store(StoreCommand),
```

with:

```rust
    Store(StoreCommand),
    /// Build the generation the host mounted for the next boot; the work runs in lmxd.
    Apply(ApplyArgs),
```

Before `StoreCommand`, the arguments. `lmx apply cancel` is a subcommand of `lmx apply`, whose own arguments it
overrides:

```rust
/// Arguments of `lmx status`.
#[derive(Debug, Args)]
pub(crate) struct StatusArgs {
    /// Wait until the guest reaches this state, then answer; the host waits so after a restart.
    #[arg(long, value_enum, requires = "generation")]
    pub(crate) wait: Option<Goal>,
    /// Generation the wait is for.
    #[arg(short, long, requires = "wait")]
    pub(crate) generation: Option<String>,
    /// Longest wait, such as 90s, 10m or 1h [default: 10m].
    #[arg(long, value_parser = duration, requires = "wait")]
    pub(crate) timeout: Option<Duration>,
    /// Output selection.
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

/// A state `lmx status --wait` waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Goal {
    /// The generation is booted, healthy and finalized.
    Converged,
}

/// Arguments of `lmx apply`.
#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub(crate) struct ApplyArgs {
    /// Another apply operation.
    #[command(subcommand)]
    pub(crate) command: Option<ApplyCommand>,
    /// Generation to apply; it must be the one the host mounted.
    #[arg(short, long, required = true)]
    pub(crate) generation: Option<String>,
    /// Follow the apply until it ends, with the build's output.
    #[arg(long)]
    pub(crate) follow: bool,
    /// Output selection.
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

/// Apply operations besides starting one.
#[derive(Debug, Subcommand)]
pub(crate) enum ApplyCommand {
    /// Cancel the apply of a generation and wait until it stops.
    Cancel(CancelArgs),
}

/// Arguments of `lmx apply cancel`.
#[derive(Debug, Args)]
pub(crate) struct CancelArgs {
    /// Generation whose apply to cancel.
    #[arg(short, long)]
    pub(crate) generation: String,
    /// Output selection.
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}
```

At the end of the file, the duration parser, with tests of it and of the new syntax:

```rust
/// Parses a duration such as `90s`, `10m` or `1h`; a number alone is seconds.
fn duration(text: &str) -> Result<Duration, String> {
    let digits = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(digits);
    let scale = match unit {
        "" | "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        _ => 0,
    };
    match number.parse::<u64>() {
        Ok(number) if number > 0 && scale > 0 => {
            Ok(Duration::from_secs(number.saturating_mul(scale)))
        }
        _ => Err("expected a duration such as 90s, 10m or 1h".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_durations_in_seconds_minutes_and_hours() {
        assert_eq!(duration("90"), Ok(Duration::from_secs(90)));
        assert_eq!(duration("90s"), Ok(Duration::from_secs(90)));
        assert_eq!(duration("10m"), Ok(Duration::from_secs(600)));
        assert_eq!(duration("1h"), Ok(Duration::from_secs(3600)));
        for invalid in ["", "0s", "m", "10 m", "1.5h", "-1s", "10d"] {
            assert!(duration(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn apply_takes_a_generation_or_a_cancel() {
        let cli = Cli::try_parse_from(["lmx", "apply", "-g", "g1", "--follow", "--json"])
            .expect("an apply");
        let Some(Command::Apply(args)) = cli.command else {
            panic!("not an apply");
        };
        assert_eq!(args.generation.as_deref(), Some("g1"));
        assert!(args.follow && args.output.json && args.command.is_none());

        let cli = Cli::try_parse_from(["lmx", "apply", "cancel", "--generation", "g1"])
            .expect("a cancel");
        let Some(Command::Apply(ApplyArgs {
            command: Some(ApplyCommand::Cancel(cancel)),
            ..
        })) = cli.command
        else {
            panic!("not a cancel");
        };
        assert_eq!(cancel.generation, "g1");

        assert!(Cli::try_parse_from(["lmx", "apply"]).is_err());
        assert!(Cli::try_parse_from(["lmx", "apply", "cancel"]).is_err());
        assert!(Cli::try_parse_from(["lmx", "status", "--wait", "converged"]).is_err());
        assert!(Cli::try_parse_from(["lmx", "status", "--timeout", "1m"]).is_err());
    }
}
```

**Step 7: Apply and cancel**

Create `crates/lmx/src/apply.rs`. With `--json`, events are JSON lines; for people, the build's lines keep their streams
and the rest goes to standard error. A cancelled apply exits with 130, and one that `lmxd` could not finish because it
stopped with 3:

```rust
//! `lmx apply`: building the generation the host mounted for the next boot.
//!
//! The apply runs in `lmxd`, which keeps going when this command is interrupted; running it again
//! attaches to the running apply. `--follow` streams its progress: with `--json` as JSON Lines that
//! end with the contract envelope, otherwise the build's output and a closing sentence.
//! `lmx apply cancel` stops it.

use std::{
    io::{self, Write},
    process::ExitCode,
};

use lmx_model::{
    Apply, ApplyEvent, ApplyPhase, ApplyState, CancelApply, Envelope, ErrorCode, OutputStream,
};

use crate::{output, owner, system::System};

/// Exit status of a cancelled apply, as of a command stopped with Ctrl-C.
const CANCELLED: u8 = 130;

/// Runs `lmx apply -g GENERATION`.
pub(crate) fn run(
    system: &System,
    generation: &str,
    follow: bool,
    json: bool,
) -> io::Result<ExitCode> {
    let answer = owner::apply(&system.owner_socket(), generation, follow, |event| {
        if json {
            output::write_json_line(&event)
        } else {
            write_event(&event)
        }
    });
    match answer {
        Ok(Ok(apply)) => {
            if json {
                output::write_json(&Envelope::success(&apply))?;
            } else {
                output::write_text(&sentence(&apply))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Ok(Err(error)) => {
            let status = match error.code {
                ErrorCode::ApplyCancelled => CANCELLED,
                ErrorCode::OwnerUnavailable => output::UNAVAILABLE,
                _ => output::FAILURE,
            };
            output::failure(json, error, status)
        }
        Err(error) => owner::report(error, json),
    }
}

/// Runs `lmx apply cancel -g GENERATION`.
pub(crate) fn cancel(system: &System, generation: &str, json: bool) -> io::Result<ExitCode> {
    match owner::cancel_apply(&system.owner_socket(), generation) {
        Ok(Ok(cancel)) => {
            if json {
                output::write_json(&Envelope::success(cancel))?;
            } else {
                output::write_text(&cancelled(generation, cancel))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Ok(Err(error)) => output::failure(json, error, output::FAILURE),
        Err(error) => owner::report(error, json),
    }
}

/// Shows an event to people: the build's lines on their own streams, the rest on standard error.
fn write_event(event: &ApplyEvent) -> io::Result<()> {
    match event {
        ApplyEvent::Output {
            stream: OutputStream::Stdout,
            line,
            ..
        } => output::write_text(&format!("{line}\n")),
        ApplyEvent::Output { line, .. } => write_stderr(line),
        ApplyEvent::Phase { phase } => write_stderr(match phase {
            ApplyPhase::Environment => "Installing the environment files.",
            ApplyPhase::Reserve => "Making room in the store.",
            ApplyPhase::Build => "Building the generation for the next boot.",
        }),
        ApplyEvent::Warning { message, .. } => write_stderr(&format!("Warning: {message}")),
        ApplyEvent::Lagged { skipped } => {
            write_stderr(&format!("lmx: {skipped} lines of the apply were skipped."))
        }
    }
}

/// Writes one line to standard error.
fn write_stderr(line: &str) -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    writeln!(stderr, "{line}")
}

/// The closing sentence of an apply for people.
fn sentence(apply: &Apply) -> String {
    let generation = &apply.generation;
    match apply.state {
        ApplyState::RestartRequired => {
            format!("Generation {generation} is built; restart the VM to boot it.\n")
        }
        ApplyState::Running => format!(
            "lmxd is applying generation {generation}; follow it with lmx apply -g {generation} \
             --follow.\n"
        ),
    }
}

/// The answer of a cancel for people.
fn cancelled(generation: &str, cancel: CancelApply) -> String {
    if cancel.cancelled {
        format!("Cancelled the apply of generation {generation}.\n")
    } else {
        format!("No apply of generation {generation} is running.\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_people_what_to_do_next() {
        let built = Apply {
            generation: "g1".into(),
            state: ApplyState::RestartRequired,
        };
        assert_eq!(
            sentence(&built),
            "Generation g1 is built; restart the VM to boot it.\n"
        );
        let running = Apply {
            state: ApplyState::Running,
            ..built
        };
        assert_eq!(
            sentence(&running),
            "lmxd is applying generation g1; follow it with lmx apply -g g1 --follow.\n"
        );
        assert_eq!(
            cancelled("g1", CancelApply { cancelled: false }),
            "No apply of generation g1 is running.\n"
        );
    }
}
```

**Step 8: Wait for convergence**

Create `crates/lmx/src/wait.rs`. It polls every two seconds and answers like `lmx status` once `Converged` holds for the
booted generation. At the timeout it reports what it saw last; a timeout beyond the clock's range waits without a
deadline:

```rust
//! `lmx status --wait converged -g GENERATION`: waiting until an update has settled.
//!
//! After the host restarts the VM into a new generation, `lmxd` checks its health and removes the
//! older generations. The host waits for that here: `lmx` asks `lmxd` every two seconds until it
//! reports `Converged` and the booted generation is the one asked for, then answers with the full
//! status. A daemon that does not answer yet, as right after the restart, is waited for.

use std::{
    io,
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use lmx_facts::generations;
use lmx_model::{CONVERGED, Condition, DEGRADED, Envelope, ErrorBody, ErrorCode, Owner};
use serde_json::{Map, Value};

use crate::{
    output,
    owner::{self, CallError},
    status,
    system::System,
};

/// Longest wait unless `--timeout` says otherwise.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Pause between two questions to `lmxd`.
const POLL: Duration = Duration::from_secs(2);

/// Waits until `generation` is converged, at most `timeout`.
///
/// At the timeout, a `Degraded` generation is `system.degraded`, a daemon that never answered is
/// `owner.unavailable`, and anything else is `wait.timeout` with the last conditions.
pub(crate) fn converged(
    system: &System,
    generation: &str,
    timeout: Duration,
    json: bool,
) -> io::Result<ExitCode> {
    let socket = system.owner_socket();
    // A timeout beyond the clock's range is no deadline at all.
    let deadline = Instant::now().checked_add(timeout);
    let mut owner = None;
    let mut unanswered = CallError::Unavailable("lmxd did not answer".into());
    loop {
        match owner::status(&socket) {
            Ok(answer) => {
                let (generations, _) = generations::read(&system.generation_paths());
                if generations.booted.as_deref() == Some(generation) && holds(&answer, CONVERGED) {
                    let status = status::collect(system);
                    if json {
                        output::write_json(&Envelope::success(&status))?;
                    } else {
                        output::write_text(&status::render(&status))?;
                    }
                    return Ok(ExitCode::SUCCESS);
                }
                owner = Some(answer);
            }
            Err(error) => unanswered = error,
        }
        let left = deadline.map_or(POLL, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        });
        if left.is_zero() {
            break;
        }
        thread::sleep(POLL.min(left));
    }

    let Some(owner) = owner else {
        return owner::report(unanswered, json);
    };
    let error = match owner
        .conditions
        .iter()
        .find(|condition| condition.kind == DEGRADED)
    {
        Some(degraded) => failure(ErrorCode::SystemDegraded, &degraded.message, &owner),
        None => failure(
            ErrorCode::WaitTimeout,
            &format!(
                "Generation {generation} did not converge within {}.",
                span(timeout)
            ),
            &owner,
        ),
    };
    let status = output::failure(json, error, output::FAILURE)?;
    if !json {
        for condition in &owner.conditions {
            eprintln!("{}", condition.message);
        }
    }
    Ok(status)
}

/// Whether `owner` reports the condition `kind`.
fn holds(owner: &Owner, kind: &str) -> bool {
    owner
        .conditions
        .iter()
        .any(|condition| condition.kind == kind)
}

/// A failure with `code` and `message`, with the conditions `owner` reported last.
fn failure(code: ErrorCode, message: &str, owner: &Owner) -> ErrorBody {
    let mut details = Map::new();
    details.insert("conditions".into(), conditions(&owner.conditions));
    ErrorBody {
        code,
        message: message.to_owned(),
        details,
    }
}

/// `conditions` as a JSON array.
fn conditions(conditions: &[Condition]) -> Value {
    serde_json::to_value(conditions).unwrap_or(Value::Null)
}

/// `duration` for people, in whole minutes when it has no seconds.
fn span(duration: Duration) -> String {
    match duration.as_secs() {
        1 => "1 second".into(),
        60 => "1 minute".into(),
        seconds if seconds % 60 == 0 => format!("{} minutes", seconds / 60),
        seconds => format!("{seconds} seconds"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_wait_in_minutes_or_seconds() {
        assert_eq!(span(TIMEOUT), "10 minutes");
        assert_eq!(span(Duration::from_secs(60)), "1 minute");
        assert_eq!(span(Duration::from_secs(90)), "90 seconds");
    }

    #[test]
    fn a_timeout_carries_the_last_conditions() {
        let owner = Owner {
            version: "0.1.0".into(),
            conditions: vec![Condition {
                kind: "RestartRequired".into(),
                message: "Generation g2 is built; restart the VM to boot it.".into(),
            }],
            operations: vec![],
        };
        let error = failure(ErrorCode::WaitTimeout, "late", &owner);
        assert_eq!(
            Value::Object(error.details),
            serde_json::json!({"conditions": [{
                "type": "RestartRequired",
                "message": "Generation g2 is built; restart the VM to boot it."
            }]})
        );
    }
}
```

In `crates/lmx/src/status.rs`, `run` starts the wait:

```rust
use crate::{cli::OutputArgs, format, layout, output, owner, system::System};

/// Runs `lmx status`.
pub(crate) fn run(system: &System, args: &OutputArgs) -> io::Result<ExitCode> {
    let status = collect(system);
    if args.json {
```

with:

```rust
use crate::{
    cli::{Goal, StatusArgs},
    format, layout, output, owner,
    system::System,
    wait,
};

/// Runs `lmx status`, or waits for a goal first with `--wait`.
pub(crate) fn run(system: &System, args: &StatusArgs) -> io::Result<ExitCode> {
    if let (Some(Goal::Converged), Some(generation)) = (args.wait, &args.generation) {
        let timeout = args.timeout.unwrap_or(wait::TIMEOUT);
        return wait::converged(system, generation, timeout, args.output.json);
    }
    let status = collect(system);
    if args.output.json {
```

**Step 9: Dispatch the commands**

In `crates/lmx/src/main.rs`, the crate documentation:

```rust
//! | `lmx store reserve`   | owner  | room in the store before the host stops the VM, in `lmxd`   |
//! | `lmx clipboard copy`  | caller | copies standard input to the Mac clipboard                  |
//! | `lmx clipboard paste` | caller | prints the Mac clipboard, if the terminal allows reads      |
//! | `lmx session NAME`    | caller | opens a named session with the selected provider            |
//!
//! Facts are read in the caller's process with the caller's privileges and need no daemon. Owner
//! operations run only in the guest owner daemon `lmxd`; without it they fail with exit status 3.
//! Caller commands act on the caller's terminal and environment, so only the caller can run them.
//!
```

with:

```rust
//! | `lmx store reserve`   | owner  | room in the store before the host stops the VM, in `lmxd`   |
//! | `lmx apply -g G`      | owner  | builds generation G for the next boot, in `lmxd`            |
//! | `lmx apply cancel`    | owner  | stops the apply of a generation                             |
//! | `lmx clipboard copy`  | caller | copies standard input to the Mac clipboard                  |
//! | `lmx clipboard paste` | caller | prints the Mac clipboard, if the terminal allows reads      |
//! | `lmx session NAME`    | caller | opens a named session with the selected provider            |
//!
//! Facts are read in the caller's process with the caller's privileges and need no daemon. Owner
//! operations run only in the guest owner daemon `lmxd`; without it they fail with exit status 3.
//! `lmx status --wait converged -g G` is one too: it answers once `lmxd` reports generation G
//! settled after a restart. Caller commands act on the caller's terminal and environment, so only
//! the caller can run them.
//!
```

the modules:

```rust
mod cli;
mod clipboard;
```

with:

```rust
mod apply;
mod cli;
mod clipboard;
```

and

```rust
mod version;
mod welcome;
```

with:

```rust
mod version;
mod wait;
mod welcome;
```

the imports:

```rust
    cli::{Cli, ClipboardCommand, Command, StoreCommand},
```

with:

```rust
    cli::{ApplyArgs, ApplyCommand, Cli, ClipboardCommand, Command, StoreCommand},
```

and the dispatch:

```rust
        Some(Command::Store(StoreCommand::Reserve(args))) => {
            store::reserve(&System::from_environment(), &args)
        }
```

with:

```rust
        Some(Command::Store(StoreCommand::Reserve(args))) => {
            store::reserve(&System::from_environment(), &args)
        }
        Some(Command::Apply(ApplyArgs {
            command: Some(ApplyCommand::Cancel(args)),
            ..
        })) => apply::cancel(
            &System::from_environment(),
            &args.generation,
            args.output.json,
        ),
        Some(Command::Apply(args)) => apply::run(
            &System::from_environment(),
            args.generation.as_deref().unwrap_or_default(),
            args.follow,
            args.output.json,
        ),
```

**Step 10: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx`

Expected: 78 tests pass, among them `a_cancel_stops_the_build_and_its_followers` and
`a_healthy_booted_generation_is_finalized_and_converges`.

---

## Task 6: Documentation

**Files:**
- Modify: `docs/contract.md`, `README.md`, `ARCHITECTURE.md`, `docs/plans/2026-10-06-guest-owner-design.md`, `docs/plans/2026-10-07-m3-apply-finalize-design.md`

The tables below are formatted by `task markdown/fix`; run it after editing, and `task ci/markdown-fmt` to check.

**Step 1: Describe the commands in the host contract**

In `docs/contract.md`: the new codes, exit status 130, the generation conditions, and sections for `lmx status --wait`,
`lmx apply` and `lmx apply cancel`. Replace

```markdown
| `3`    | The guest owner daemon is unavailable                                                   |
| `130`  | Cancelled                                                                               |
```

with:

```markdown
| `3`    | The guest owner daemon is unavailable                                                   |
| `130`  | The apply was cancelled (`apply.cancelled`)                                             |
```

and

```markdown
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
```

with:

```markdown
| Code                       | Meaning                                                            |
| -------------------------- | ------------------------------------------------------------------ |
| `owner.unavailable`        | `lmxd` is not reachable                                            |
| `apply.build_failed`       | `nixos-rebuild` failed for the requested generation                |
| `apply.cancelled`          | The operation was cancelled by an explicit request                 |
| `apply.environment_failed` | The environment files of the generation could not be installed     |
| `disk.low`                 | Free bytes or inodes are below the platform minimum                |
| `disk.unreadable`          | The usage of the store file system cannot be read                  |
| `network.unreachable`      | A required destination, such as the binary cache, is unreachable   |
| `permission.denied`        | The caller is not allowed to run the operation                     |
| `generation.mismatch`      | The mounted inputs belong to a different generation than requested |
| `contract.unsupported`     | The caller requested a contract version this binary does not speak |
| `system.degraded`          | The booted generation failed its health check                      |
| `wait.timeout`             | A wait ended before its condition held                             |
```

and

```markdown
`lmx status` and `lmx version` do not use these codes or the exit statuses `3` and `130`.
`lmx store reserve` uses exit status `3` when `lmxd` is unavailable.
```

with:

```markdown
`lmx status` without `--wait` and `lmx version` do not use these codes or the exit statuses `3` and `130`.
Owner operations, `lmx store reserve`, `lmx apply`, `lmx apply cancel` and `lmx status --wait`, use exit status `3` when `lmxd` is unavailable.
Only `lmx apply` uses exit status `130`.
```

and

```markdown
`owner` is `null` with an `owner` problem when `lmxd` does not answer within two seconds; the other facts are still answered.
Its conditions are computed when it is asked: `DiskLow` means less than the platform minimum of the store disk is free.
An operation's `created_at` is when it was requested, in Unix milliseconds.
```

with:

```markdown
`owner` is `null` with an `owner` problem when `lmxd` does not answer within two seconds; the other facts are still answered.
Its conditions are computed when it is asked:

| Condition         | When                                                                                                       |
| ----------------- | ---------------------------------------------------------------------------------------------------------- |
| `OutOfDate`       | The mounted generation is not built: an apply is needed                                                    |
| `RestartRequired` | The mounted generation is built but not booted: a restart is needed                                        |
| `Degraded`        | The mounted generation is booted and failed its last health check; the message gives the reason            |
| `Converged`       | The mounted generation is booted, healthy and finalized: older generations and their boot entries are gone |
| `DiskLow`         | Less than the platform minimum of the store disk is free                                                   |

Without a mounted generation there is no generation condition.
A booted generation that is healthy and still keeps older generations has none either: `lmxd` is about to remove them, and a `SystemFinalize` operation shows it.
An operation's `created_at` is when it was requested, in Unix milliseconds.
```

and

```markdown
## `lmx version`
```

with:

```markdown
## `lmx status --wait converged -g GENERATION`

The host runs it after it restarts the VM into a new generation.
`lmx` asks `lmxd` every two seconds until it reports `Converged` and the booted generation is `GENERATION`, then answers as `lmx status` does.
A daemon that does not answer yet, as right after the restart, is waited for.
`--timeout` limits the wait, such as `90s`, `10m` or `1h`; the default is 10 minutes.

| Error code          | When                                                                       |
| ------------------- | -------------------------------------------------------------------------- |
| `system.degraded`   | At the timeout, the generation is `Degraded`; the message gives the reason |
| `wait.timeout`      | At the timeout, the generation is not converged for another reason         |
| `owner.unavailable` | `lmxd` never answered; exit status `3`                                     |

`details` of `system.degraded` and `wait.timeout` has `conditions`, the conditions `lmxd` reported last.

## `lmx apply -g GENERATION`

Root only. The host runs it after it mounts a generation at `/mnt/limanix`.
`lmxd` installs the generation's environment files into `/etc/limanix`, makes room in the store as `lmx store reserve` does, and builds the generation for the next boot with `nixos-rebuild boot`.
The apply belongs to `lmxd`: interrupting the command does not stop it, and running the command again attaches to the running apply.
An apply of another generation is cancelled and replaced.

Without `--follow`, the answer comes at once.
`data` has `generation` and `state`: `running`, or `restart_required` when the generation is built.

With `--follow`, the command waits for the outcome.
`--json` then writes JSON Lines: one line per event, and the envelope last.

| `event`   | Fields                                                                                                   |
| --------- | -------------------------------------------------------------------------------------------------------- |
| `phase`   | `phase`: `environment`, `reserve` or `build`                                                             |
| `output`  | `stream` (`stdout` or `stderr`) and `line`, a line of the build; `truncated: true` when the line was cut |
| `warning` | `code` and `message` of a problem that does not stop the apply, such as `disk.low` from the reserve      |
| `lagged`  | `skipped`: the follower fell behind and missed that many events                                          |

A follower that attaches to a running apply receives the events from then on, and always the outcome.
On success the envelope's `state` is `restart_required`: restart the VM to boot the generation.

| Error code                 | When                                                                                                                         |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| `generation.mismatch`      | `GENERATION` is not the mounted generation; `details` has `requested` and `mounted` (`null` without a mount)                 |
| `apply.environment_failed` | The environment files cannot be installed                                                                                    |
| `apply.build_failed`       | `nixos-rebuild` failed; `details` has `exit_code` when it exited, and `disk` with the store disk usage when the disk is full |
| `apply.cancelled`          | `lmx apply cancel`, an apply of another generation, or a cancel through the Task API stopped the apply; exit status `130`    |
| `permission.denied`        | The caller is not root                                                                                                       |
| `owner.unavailable`        | `lmxd` cannot be reached, or it stopped before the apply ended; exit status `3`                                              |

Without `--json`, the command prints the build's lines on their own streams and a closing sentence.

Examples: [built](../contract/v1/apply-restart-required.json), [build failed](../contract/v1/apply-build-failed.json), [followed](../contract/v1/apply-follow.jsonl).

## `lmx apply cancel -g GENERATION`

Root only. The host runs it when the person stops an update.
`lmxd` cancels the apply of `GENERATION` and answers once it has stopped; its followers receive `apply.cancelled`.
A build is killed; a boot loader update that `nixos-rebuild` started finishes in its own unit.

`data` has `cancelled`: `true` when an apply of the generation was running, `false` when none was.

| Error code          | When                                      |
| ------------------- | ----------------------------------------- |
| `permission.denied` | The caller is not root                    |
| `owner.unavailable` | `lmxd` cannot be reached; exit status `3` |

## `lmx version`
```

and

```markdown
  Hosts treat an unknown code as a generic failure.
- Renaming, removing or changing the meaning of a field needs a new contract version.
```

with:

```markdown
  Hosts treat an unknown code as a generic failure.
- A new `event` of `lmx apply --follow` is compatible too: hosts skip events they do not know.
  A new phase or state needs a new contract version.
- Renaming, removing or changing the meaning of a field needs a new contract version.
```

**Step 2: Describe the update in the README and the contributor map**

In `README.md`, replace

```markdown
It reports what the guest really is and answers the LimaNix host with versioned JSON over management SSH.
Its daemon, `lmxd`, owns the work that must not depend on a caller's session, starting with room in the Nix store.
```

with:

```markdown
It reports what the guest really is and answers the LimaNix host with versioned JSON over management SSH.
Its daemon, `lmxd`, owns the work that must not depend on a caller's session: room in the Nix store, and updates of the system from build to finalize.
```

and

```markdown
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
```

with:

```markdown
| Command                 | Answers or does                                                                                                |
| ----------------------- | -------------------------------------------------------------------------------------------------------------- |
| `lmx help`              | The workspace and the commands inside the VM and on the Mac; also `lmx`, `lmx -h`                              |
| `lmx info`              | The kernel, guest disk, shared folders and failed units                                                        |
| `lmx welcome`           | The summary an interactive shell prints when it starts                                                         |
| `lmx status`            | Desired, built and booted generations; store disk usage; interfaces; failed systemd units; the state of `lmxd` |
| `lmx version`           | The binary version and the host contract version                                                               |
| `lmx store reserve`     | Collects unreferenced store paths when space is low; run in `lmxd`, root only                                  |
| `lmx apply -g G`        | Builds the mounted generation G for the next boot; run in `lmxd`, root only; `--follow` streams the build      |
| `lmx apply cancel -g G` | Stops the apply of generation G; root only                                                                     |
| `lmx clipboard copy`    | Copies standard input to the Mac clipboard                                                                     |
| `lmx clipboard paste`   | Prints the Mac clipboard, if the terminal allows reads                                                         |
| `lmx session NAME`      | Opens a named session with the provider that the selected modules configure                                    |
```

and

```markdown
Add `--json` to `status`, `version` and `store reserve` to answer with the [host contract](docs/contract.md).
`lmx status` reads every fact independently: an unreadable fact is reported as a problem, and the others are still answered.
```

with:

```markdown
Add `--json` to `status`, `version`, `store reserve`, `apply` and `apply cancel` to answer with the [host contract](docs/contract.md).
`lmx status --wait converged -g G` answers once `lmxd` reports generation G booted, healthy and finalized.
`lmx status` reads every fact independently: an unreadable fact is reported as a problem, and the others are still answered.
```

and

```markdown
`lmxd` runs as root from systemd, started at boot and through `lmx.socket`, with readiness and a watchdog.
It replaces the platform's store guard, its timer, the daily `nix-gc` timer and the host's reserve over SSH:
```

with:

```markdown
`lmxd` runs as root from systemd, started at boot and through `lmx.socket`, with readiness and a watchdog.
It replaces the platform's store guard, its timer, the daily `nix-gc` timer, and the host's reserve, rebuild and prune over SSH:
```

and

```markdown
- `lmx store reserve` does the same at once for the host and answers with the usage before and after;
- every operation is a Solti task, so its output reaches the journal (`journalctl -u lmx`).
```

with:

```markdown
- `lmx store reserve` does the same at once for the host and answers with the usage before and after;
- `lmx apply` installs the environment files of the generation the host mounted, makes room in the store and runs `nixos-rebuild boot`; a caller that disconnects leaves the apply running, and asking again attaches to it;
- after the restart into a new generation, it checks the platform units, the shared folders and a login shell of the development account, then removes the older generations and rewrites the boot entries;
- every operation is a Solti task, so its output reaches the journal (`journalctl -u lmx`).
```

and

```markdown
- every operation is a Solti task, so its output reaches the journal (`journalctl -u lmx`).
```

with:

```markdown
- every operation is a Solti task, so its output reaches the journal (`journalctl -u lmx`).

For an update, the host stops the system daemon and starts one from the mounted generation with `lmxd --transient`, which applies but neither guards the store nor finalizes.
A daemon never takes over a socket that another daemon still serves.
```

In `ARCHITECTURE.md`, replace

````markdown
                                                                      ├──► store guard and reserve
                                                                      └──► Solti tasks ──► nix-store
```
````

with:

````markdown
                                                                      ├──► store guard and reserve
                                                                      ├──► apply, health and finalize
                                                                      └──► Solti tasks ──► nix-store, nixos-rebuild,
                                                                                           nix-env, systemctl, sudo,
                                                                                           systemd-run
```
````

and

```markdown
- `lmx-ipc` and `lmx` do not depend on Solti; only `lmxd` does.
- `lmxd` runs programs only as Solti tasks, never with `std::process`: tools by their absolute paths in the configuration, and `/bin/sh` for the roots report.
- Only `lmxd` creates its tasks. The Task API on its socket reads them, and root may cancel or delete them.
```

with:

```markdown
- `lmx-ipc` and `lmx` do not depend on Solti; only `lmxd` does.
- `lmxd` runs programs only as Solti tasks, never with `std::process`: tools by their absolute paths in the configuration, and `/bin/sh` for the roots report, the health check and finalize.
- An apply's state lives in `lmxd`'s memory; after a restart the generation markers are the truth. Finalize runs only in the daemon of the booted system, never in `lmxd --transient`.
- One daemon serves the socket at a time: `lmxd` refuses a socket that another daemon still serves, so two daemons never apply at once.
- Only `lmxd` creates its tasks. The Task API on its socket reads them, and root may cancel or delete them.
```

and

```markdown
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
```

with:

```markdown
| Area                | Responsibility                                                                    | Start here                                                                    |
| ------------------- | --------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| Contract types      | Configuration, envelope, error codes, status, apply and version                   | [`lmx-model/src/lib.rs`](crates/lmx-model/src/lib.rs)                         |
| Fact readers        | Disk, generations, machine, mounts, network and failed units                      | [`lmx-facts/src/lib.rs`](crates/lmx-facts/src/lib.rs)                         |
| Command line        | Commands, other names, output selection and exit codes                            | [`lmx/src/main.rs`](crates/lmx/src/main.rs)                                   |
| Status              | Collecting facts and rendering them                                               | [`lmx/src/status.rs`](crates/lmx/src/status.rs)                               |
| Guest pages         | Help, info and the welcome for people in the guest                                | [`lmx/src/welcome.rs`](crates/lmx/src/welcome.rs)                             |
| Terminal text       | Columns, wrapping and the palette                                                 | [`lmx/src/layout.rs`](crates/lmx/src/layout.rs)                               |
| Caller commands     | The clipboard through the terminal or tmux, and named sessions                    | [`lmx/src/clipboard.rs`](crates/lmx/src/clipboard.rs)                         |
| Owner calls         | `lmx store reserve`, `lmx apply`, and the owner part and the wait of `lmx status` | [`lmx/src/owner.rs`](crates/lmx/src/owner.rs)                                 |
| IPC                 | The `lmx.v1.Owner` protocol and its Unix-socket client                            | [`lmx-ipc/proto/lmx/v1/owner.proto`](crates/lmx-ipc/proto/lmx/v1/owner.proto) |
| Daemon              | Startup, serving the socket, systemd and the stopping order                       | [`lmxd/src/main.rs`](crates/lmxd/src/main.rs)                                 |
| Store domain        | The guard, reserve, conditions and the store tasks                                | [`lmxd/src/store.rs`](crates/lmxd/src/store.rs)                               |
| Apply               | Environment files, reserve and build of a mounted generation, and its followers   | [`lmxd/src/apply.rs`](crates/lmxd/src/apply.rs)                               |
| Generation observer | Health check, finalize and the generation conditions                              | [`lmxd/src/observer.rs`](crates/lmxd/src/observer.rs)                         |
| Contract examples   | Published answers of each contract version                                        | [`contract/v1/`](contract/v1)                                                 |
```

**Step 3: Record the implementation in the designs**

In `docs/plans/2026-10-06-guest-owner-design.md`, replace

```markdown
in the Nix store and answers `lmx store reserve`. See [the M2 design](2026-10-07-m2-lmxd-store-design.md)
and [the M2 plan](2026-10-07-m2-lmxd-store.md).
```

with:

```markdown
in the Nix store and answers `lmx store reserve`. See [the M2 design](2026-10-07-m2-lmxd-store-design.md)
and [the M2 plan](2026-10-07-m2-lmxd-store.md). M3 is implemented in this repository: apply, cancel,
the transient daemon, the health check, finalize and the converged wait; the host switches to them in
M1c. See [the M3 design](2026-10-07-m3-apply-finalize-design.md) and
[the M3 plan](2026-10-07-m3-apply-finalize.md).
```

and

```markdown
|---|---|---|---|
| `SystemApply{G}` | Check the mounted G → install ENV → reserve through `StoreCollect` → `nixos-rebuild boot` | `system`, replace | Only by the host |
| `SystemFinalize{G}` | Health check → delete older generations → `switch-to-configuration boot` → collect | `system` | Automatically when desired = built = booted and there is something to finalize |
```

with:

```markdown
|---|---|---|---|
| `SystemApply{G}` | Check the mounted G → install ENV → reserve through `StoreCollect` → `nixos-rebuild boot` | `system`, queued | Only by the host |
| `SystemFinalize{G}` | Health check → delete older generations → `switch-to-configuration boot` → collect | `system` | Automatically when desired = built = booted and there is something to finalize |
```

In `docs/plans/2026-10-07-m3-apply-finalize-design.md`, the status and what the implementation settled: the queued
build, the timeout of a check, `--root`, `state` in the follow example, and a finalize that must finish before
`Converged`. Replace

```markdown
Status: design agreed on 2026-10-07. It refines M3 of
[the guest owner design](2026-10-06-guest-owner-design.md) (sections 6–9 and 12) and builds on
```

with:

```markdown
Status: design agreed on 2026-10-07 and implemented in this repository; see
[the M3 plan](2026-10-07-m3-apply-finalize.md). Host integration follows in M1c. It refines M3 of
[the guest owner design](2026-10-06-guest-owner-design.md) (sections 6–9 and 12) and builds on
```

and

```markdown
  so it works on the first create, before the platform exists.
- `tools.nixos_rebuild`, `tools.nix_env`, `tools.sudo` and `tools.bash`: absolute store paths.
- `health.units`: the platform units the health check requires, such as `sshd.service`,
```

with:

```markdown
  so it works on the first create, before the platform exists.
- `tools.nixos_rebuild`, `tools.nix_env`, `tools.sudo`, `tools.bash` and `tools.systemd_run`:
  absolute store paths.
- `health.units`: the platform units the health check requires, such as `sshd.service`,
```

and

```markdown
|---|---|---|
| `SystemApply` | `nixos-rebuild boot --flake path:/mnt/limanix/flake#runtime --no-write-lock-file --no-update-lock-file` | `system`, replace |
| `SystemHealth` | the health script (below) | `health` |
```

with:

```markdown
|---|---|---|
| `SystemApply` | `nixos-rebuild boot --flake path:/mnt/limanix/flake#runtime --no-write-lock-file --no-update-lock-file` | `system`, queued |
| `SystemHealth` | the health script (below) | `health` |
```

and

```markdown
| `SystemHealth` | the health script (below) | `health` |
| `SystemFinalize` | `nix-env --profile /nix/var/nix/profiles/system --delete-generations old`, then, if that worked, `switch-to-configuration boot` | `system` |
| `StoreCollect`, `StoreRoots` | as in M2 | `store` |
```

with:

```markdown
| `SystemHealth` | the health script (below) | `health` |
| `SystemFinalize` | `nix-env --profile /nix/var/nix/profiles/system --delete-generations old`, then, if that worked, `switch-to-configuration boot` in its own unit through `systemd-run`, as `nixos-rebuild` runs it | `system`, dropped while a build runs |
| `StoreCollect`, `StoreRoots` | as in M2 | `store` |
```

and

```markdown
| `Degraded` | desired, built and booted are equal, and the last health check failed. The message holds the reason, and older generations stay for a rollback. |
| `Converged` | desired, built and booted are equal, the system is healthy, and the profile has one generation |
| `DiskLow` | as in M2 |
```

with:

```markdown
| `Degraded` | desired, built and booted are equal, and the last health check failed. The message holds the reason, and older generations stay for a rollback. |
| `Converged` | desired, built and booted are equal, the system is healthy, the profile has one generation, and no finalize runs or waits for a retry |
| `DiskLow` | as in M2 |
```

and

```markdown
- **Result.** Exit status 0 means healthy. Otherwise the task's output gives the reason, such as
  `sshd.service is not active`.
```

with:

```markdown
- **Result.** Exit status 0 means healthy. Otherwise the task's output gives the reason, such as
  `sshd.service is not active`. A check that runs longer than 2 minutes, such as on a stuck mount,
  fails.
```

and

```markdown
1. **Checks:**
   - mounted desired differs from G, or is unknown: `generation.mismatch`, with details
```

with:

```markdown
1. **Checks,** in this order:
   - mounted desired differs from G, or is unknown: `generation.mismatch`, with details
```

and

```markdown
     `{requested, mounted}`;
   - built equals G: the result is `restart_required` at once;
```

with:

```markdown
     `{requested, mounted}`;
   - an apply of G is running: the caller attaches to it;
   - an apply of another generation is running: `lmxd` cancels it, waits until it stops, and checks
     again. The host mounted another generation, so the older build must not finish after the answer,
     even when G is built;
   - built equals G: the result is `restart_required` at once;
```

and

```markdown
   - built equals G: the result is `restart_required` at once;
   - a build of G is running: the caller attaches to it;
   - a build of another generation is running: it is replaced.
2. **Phases:**
```

with:

```markdown
   - built equals G: the result is `restart_required` at once;
   - otherwise a new apply starts. Its build queues in the `system` slot, so a finalize that runs
     there finishes first instead of being killed. Right before the build, the mounted generation is
     read again; one that changed is a mismatch.
2. **Phases:**
```

and

```markdown
   | non-zero exit | `apply.build_failed`, details `{exit_code}`. If usage is below the minimum or the output contains `No space left on device`, the details also hold the disk usage. |
   | cancelled | `apply.cancelled`, exit status 130 |
```

with:

```markdown
   | non-zero exit | `apply.build_failed`, details `{exit_code}`. If usage is below the minimum or the output contains `No space left on device`, the details also hold the disk usage. |
   | cancelled by `lmx apply cancel`, by an apply of another generation, or through the Task API | `apply.cancelled`, exit status 130 |
   | `lmxd` stops before the apply ends | `owner.unavailable`, exit status 3 |
```

and

````markdown
{"event":"lagged","skipped":12}
{"contract":1,"ok":true,"data":{"generation":"0123456789ab","outcome":"restart_required"}}
```
````

with:

````markdown
{"event":"lagged","skipped":12}
{"contract":1,"ok":true,"data":{"generation":"0123456789ab","state":"restart_required"}}
```
````

and

```markdown
2. If healthy, it runs `SystemFinalize`, then `StoreCollect` at idle priority. Garbage collection
   continues in the background: `Converged` needs only the older generations gone.
3. If not healthy, the system is `Degraded`. The older generations stay, and the check repeats every
```

with:

```markdown
2. If healthy, it runs `SystemFinalize`, then `StoreCollect` at idle priority. Garbage collection
   continues in the background: `Converged` needs the finalize, not the collection.
3. If not healthy, the system is `Degraded`. The older generations stay, and the check repeats every
```

and

```markdown
   minute.
4. A failed finalize is logged and retried at most every 15 minutes.
```

with:

```markdown
   minute.
4. A failed finalize is logged and retried at most every 15 minutes, even after it removed the older
   generations, so the boot entries get rewritten. While a finalize runs or waits for its retry, the
   generation is not `Converged`.
```

and

```markdown
  - it lives until the host stops it or the VM restarts.
- **The system daemon** can also apply. It never races finalize: during an apply desired differs
  from built, and finalize needs all three equal.
- **System root.** For tests, `Options` gains a system root for every path:
  - `/mnt/limanix`;
```

with:

```markdown
  - it lives until the host stops it or the VM restarts.
- **One daemon at a time.** A daemon refuses a socket that another daemon still serves, so the
  host stops the system daemon before it starts a transient one, and two daemons never apply at
  once.
- **The system daemon** can also apply. A finalize reads the markers again after its health check,
  and it is dropped while a build holds the `system` slot: after that build, the older generations
  would include the booted one. A build queues behind a running finalize.
- **System tasks** run with `PATH=/run/current-system/sw/bin`, as the host's login shell had it.
- **System root.** For tests, `Options` gains a system root for every path; the binary takes it
  from the hidden flag `--root`:
  - `/mnt/limanix`;
```

and

```markdown
## Decisions
```

with:

```markdown
## Known gaps

- A finalize that keeps failing is only logged and retried every 15 minutes. Meanwhile there is no
  generation condition, so `lmx status --wait converged` ends with `wait.timeout` after its timeout.
  M1c decides how the host reports it.

## Decisions
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

Expected: every step passes, and the four binaries are static.

**Markdown.** `task ci/markdown-fmt` passes.

**Guest.** In the personal LimaNix VM, harmless parts only, as transient units. A real apply or finalize would change that VM's system, so the daemons get a system root `/tmp/lmx-m3/root` with generation markers, one profile generation, and `proc` linked to `/proc`. The configuration names the real tools, except a fake `nixos-rebuild` that copies the desired marker, and disk thresholds of 1% so nothing is collected:

1. **Convergence.** With desired, built and booted `g1`, the system daemon (`lmxd --config … --socket /tmp/lmx-m3/root/run/lmx/lmx.sock --root /tmp/lmx-m3/root`) checks the units with the real `systemctl` and runs `true` in the dev user's login shell through the real `sudo`; `LMX_SYSTEM_ROOT=/tmp/lmx-m3/root lmx status --wait converged -g g1 --json` succeeds.
1. **Degraded.** With a unit that does not exist in `health.units`, the wait ends with `system.degraded`, naming the unit.
1. **Apply.** With desired `g2` and built `g1`, `lmxd --transient` installs `/tmp/lmx-m3/root/etc/limanix/environment` with mode `0640` and the dev user's group, runs the fake build with `PATH` and the user's variables, and answers `restart_required`; `lmx apply -g g3` answers `generation.mismatch`, and `nobody` gets `permission.denied`.
1. **Cancel.** With a build that hangs, `lmx apply cancel` answers `cancelled: true`, the follower exits with 130 and `apply.cancelled`, and the build's process group is gone.
1. **One daemon.** A second daemon on the served socket refuses to start with `another lmxd serves …`, and the first still answers.
1. **Stop.** Stopping the transient daemon during a build gives its follower `owner.unavailable` and exit status 3.

Remove the units and `/tmp/lmx-m3` afterwards. The whole path, update to `Converged`, runs in M1c on a throwaway VM.

## Suggested commits

The user commits. One commit per task keeps every commit building and green:
1. `feat(model): apply answers, generation conditions and update configuration`
1. `feat(facts): generations of the system profile`
1. `feat(lmxd): apply a mounted generation through Owner.Apply and Owner.CancelApply`
1. `feat(lmxd): health check, finalize and generation conditions`
1. `feat(lmx): apply, apply cancel and status --wait converged`
1. `docs: M3 design, plan, contract and contributor map`
