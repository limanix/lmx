# M1a: lmx Foundation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create the `limanix/lmx` Rust workspace with the host-contract model, the guest fact readers,
`lmx status` and `lmx version`, containerized checks, and a static musl release pipeline.

**Architecture:** One Cargo workspace with three crates:
- `lmx-model`: the values that cross a boundary. It holds the configuration written by NixOS and the JSON
  envelope and status the host reads. It does no I/O except loading the configuration.
- `lmx-facts`: readers of the running guest: `statvfs`, generation markers, `ip -j address show`,
  `systemctl list-units`. Each reader splits I/O from a pure parser.
- `lmx`: the binary (clap). It renders text for people or a versioned JSON envelope for the host.

Checks and the release build run through `mr-chelyshkin/tasks` Taskfiles in the `ci/rust` image. GitHub workflows
call them through `mr-chelyshkin/actions/invoke-taskfile`.

**Tech Stack:**
- Rust 1.90.0, edition 2024
- clap 4.6, serde 1.0, serde_json 1.0, rustix 1.1, thiserror 2.0, tempfile 3 (tests)
- Task 3.53.1 with `mr-chelyshkin/tasks` v0.0.5, Docker, GitHub Actions

---

## Before you start

- **Design.** [`2026-10-06-guest-owner-design.md`](2026-10-06-guest-owner-design.md), sections 4, 5, 8 and 11.
  This plan is milestone **M1a**:
  - M1b covers help, info, welcome, clipboard, sessions and multi-call names.
  - M1c covers the client and the platform.
  - Both have separate plans.
- **Repository.** `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx` (remote `git@github.com:limanix/lmx.git`).
- **Shell quirk.** `cd` into the limanix directories triggers a zsh GVM hook that prints `GVM_ROOT not set` and can
  swallow output. Use absolute paths, `git -C`, `task --dir` and `cargo <subcommand> --manifest-path` instead of `cd`.
  - `--manifest-path` follows the subcommand.
  - rustup picks the toolchain from the current directory, so commands name it explicitly: `cargo +1.90.0`.
- **Fast local loop.** `cargo` 1.90.0 is installed on the host.
- **Release-equivalent checks.** `task` runs Cargo in `ghcr.io/mr-chelyshkin/ci/rust:1.90.0`. It needs Docker,
  and `--yes` trusts the remote Taskfiles.
- **Markdown has no front matter.** The CI formatter (`mdformat`) turns a YAML block into a heading.
- **Code style follows `/Users/igoss/Desktop/projects/solti/taskvisor`:**
  - every crate root starts with `//!` documentation that states its purpose and shows a table of its parts;
  - every module starts with `//!` documentation;
  - every item, including private struct fields, has a `///` comment;
  - every crate root has `#![forbid(unsafe_code)]`;
  - test names are sentences about behavior;
  - manifests align `=` and pin full versions.
- **Lints.** The workspace warns on the following, and CI denies warnings, so any of them fails CI:
  - missing docs on public items (`missing_docs`);
  - missing docs on private items (`clippy::missing_docs_in_private_items`);
  - unreachable `pub` (`unreachable_pub`);
  - public types without `Debug` (`missing_debug_implementations`).
- **Commits are made by the user.** Each task's "Commit" step names the files and suggests a message.
  Implementers stop there and leave the changes uncommitted. Never commit, push or open a pull request.

---

## Task 0: Branch and repository hygiene

**Files:**
- Modify: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/.gitignore`
- Add: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/docs/plans/2026-10-06-guest-owner-design.md`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/docs/plans/2026-10-06-m1a-lmx-foundation.md` (already written, untracked)

**Step 1: Create the branch**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx switch -c feat/m1a-foundation
```

**Step 2: Ignore task caches, release output, macOS metadata and the IDE folder**

Append this block to `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/.gitignore` after a blank line. `target`
is already listed. `.task/` is Task's cache of remote Taskfiles.

```gitignore
.cache/
.task/
dist/
.DS_Store
```

In the existing JetBrains section, uncomment `#.idea/` to `.idea/`: the project is opened in RustRover, and the
sibling repos ignore `.idea/`.

**Step 3: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add .gitignore docs/plans
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Add guest owner design and M1a plan"
```

---

## Task 1: Workspace skeleton

**Files:**
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/rust-toolchain.toml`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/.cargo/config.toml`
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-model/Cargo.toml`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-model/src/lib.rs`
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-facts/Cargo.toml`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-facts/src/lib.rs`
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx/Cargo.toml`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx/src/main.rs`

**Step 1: Write the workspace manifest** (`Cargo.toml`)

```toml
[workspace]
resolver = "3"
members  = ["crates/lmx", "crates/lmx-facts", "crates/lmx-model"]

[workspace.package]
version      = "0.1.0"
edition      = "2024"
rust-version = "1.90.0"
license      = "Apache-2.0"
repository   = "https://github.com/limanix/lmx"
homepage     = "https://limanix.dev"

[workspace.dependencies]
lmx-facts  = { path = "crates/lmx-facts" }
lmx-model  = { path = "crates/lmx-model" }
clap       = { version = "4.6.6", features = ["derive"] }
rustix     = { version = "1.1.4", features = ["fs"] }
serde      = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.151"
tempfile   = "3.27.0"
thiserror  = "2.0.20"

[workspace.lints.rust]
missing_debug_implementations = "warn"
missing_docs                  = "warn"
unreachable_pub               = "warn"
unsafe_code                   = "forbid"

[workspace.lints.clippy]
missing_docs_in_private_items = "warn"

# Panics unwind: lmxd relies on Taskvisor catching task panics, and Cargo cannot set `panic` per package.
[profile.release]
codegen-units = 1
lto           = true
strip         = true
```

**Step 2: Pin the toolchain** (`rust-toolchain.toml`)

```toml
# Keep equal to `rust-version` in Cargo.toml, with three components: the shared Taskfile
# derives the CI image tag and RUSTUP_TOOLCHAIN from that field.
[toolchain]
channel    = "1.90.0"
components = ["clippy", "rustfmt"]
```

**Step 3: Link musl targets with `rust-lld`** (`.cargo/config.toml`)

```toml
# Release binaries are static musl executables. rust-lld ships with the toolchain,
# so both architectures link on any build host without a C cross toolchain.
# That holds while dependencies are pure Rust: a crate that compiles C (ring,
# zstd-sys) needs a musl C compiler, which the CI image lacks.
[target.aarch64-unknown-linux-musl]
linker = "rust-lld"

[target.x86_64-unknown-linux-musl]
linker = "rust-lld"
```

**Step 4: Write the crate manifests**

`crates/lmx-model/Cargo.toml`:

```toml
[package]
name         = "lmx-model"
description  = "Data contracts shared by the lmx guest binaries and the LimaNix host"
version.workspace      = true
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
repository.workspace   = true
homepage.workspace     = true
publish = false

[dependencies]
serde      = { workspace = true }
serde_json = { workspace = true }
thiserror  = { workspace = true }

[lints]
workspace = true
```

`crates/lmx-facts/Cargo.toml`:

```toml
[package]
name         = "lmx-facts"
description  = "Readers of the running LimaNix guest"
version.workspace      = true
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
repository.workspace   = true
homepage.workspace     = true
publish = false

[dependencies]
lmx-model  = { workspace = true }
rustix     = { workspace = true }
serde      = { workspace = true }
serde_json = { workspace = true }
thiserror  = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }

[lints]
workspace = true
```

`crates/lmx/Cargo.toml`:

```toml
[package]
name         = "lmx"
description  = "Guest owner command of a LimaNix VM"
version.workspace      = true
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
repository.workspace   = true
homepage.workspace     = true
publish = false

[dependencies]
clap       = { workspace = true }
lmx-facts  = { workspace = true }
lmx-model  = { workspace = true }
serde      = { workspace = true }
serde_json = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }

[lints]
workspace = true
```

**Step 5: Add placeholder crate roots**

`crates/lmx-model/src/lib.rs` and `crates/lmx-facts/src/lib.rs`. Use the matching crate name in each heading:

```rust
//! # lmx-model
#![forbid(unsafe_code)]
```

`crates/lmx/src/main.rs`:

```rust
//! # lmx
#![forbid(unsafe_code)]

fn main() {}
```

**Step 6: Verify the workspace resolves and builds**

```bash
cargo +1.90.0 check --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml --workspace
```

Expected: `Finished` with no warnings. `Cargo.lock` is created.

**Step 7: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add Cargo.toml Cargo.lock rust-toolchain.toml .cargo crates
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Create lmx workspace"
```

---

## Task 2: Platform configuration (`lmx-model::Config`)

NixOS writes `/etc/lmx/config.json`. M1c generates it, and `lmx` reads it. Unknown fields are rejected: the file and
the binary come from one generation, so a mismatch is a packaging error.

**Files:**
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-model/src/config.rs`
- Modify: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-model/src/lib.rs`

**Step 1: Write the failing tests**

Create `crates/lmx-model/src/config.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Configuration as the platform renders it for schema 1.
    const SAMPLE: &str = r#"{
        "schema": 1,
        "vm": {"name": "dev-box", "arch": "arm64", "system": "NixOS 26.05"},
        "generation": "0123456789ab",
        "user": {"name": "dev", "home": "/home/dev", "uid": 501},
        "modules": ["lmx:console", "lmx:go"],
        "disk": {"collect_percent": 20, "minimum_percent": 10},
        "session": {"command": null, "providers": ["lmx:tmux"]},
        "tools": {"ip": "/run/current-system/sw/bin/ip", "systemctl": "/run/current-system/sw/bin/systemctl"}
    }"#;

    #[test]
    fn parses_the_platform_configuration() {
        let config = Config::from_json(SAMPLE.as_bytes()).expect("valid configuration");
        assert_eq!(config.vm.name, "dev-box");
        assert_eq!(config.generation, "0123456789ab");
        assert_eq!(config.user.uid, 501);
        assert_eq!(config.disk.minimum_percent, 10);
        assert_eq!(config.session.command, None);
    }

    #[test]
    fn rejects_another_schema() {
        let other = SAMPLE.replace("\"schema\": 1", "\"schema\": 2");
        let error = Config::from_json(other.as_bytes()).expect_err("schema 2 is unknown");
        assert!(matches!(error, ConfigError::Schema { found: 2 }), "{error}");
    }

    #[test]
    fn reports_another_schema_before_its_fields() {
        let other = SAMPLE.replace("\"schema\": 1,", "\"schema\": 2, \"theme\": \"mocha\",");
        let error = Config::from_json(other.as_bytes()).expect_err("schema 2 is unknown");
        assert!(matches!(error, ConfigError::Schema { found: 2 }), "{error}");
    }

    #[test]
    fn rejects_unknown_fields() {
        let other = SAMPLE.replace("\"schema\": 1,", "\"schema\": 1, \"extra\": true,");
        let error =
            Config::from_json(other.as_bytes()).expect_err("unknown fields are packaging errors");
        assert!(matches!(error, ConfigError::Invalid(_)), "{error}");
    }

    #[test]
    fn reports_the_path_of_an_unreadable_file() {
        let error =
            Config::load(Path::new("/nonexistent/lmx/config.json")).expect_err("missing file");
        assert!(
            error.to_string().contains("/nonexistent/lmx/config.json"),
            "{error}"
        );
    }
}
```

Register it in `crates/lmx-model/src/lib.rs`:

```rust
//! # lmx-model
#![forbid(unsafe_code)]

mod config;

pub use config::{
    CONFIG_PATH, CONFIG_SCHEMA, Config, ConfigError, DiskPolicy, Session, Tools, User, Vm,
};
```

**Step 2: Run the tests to verify they fail**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model config
```

Expected: FAIL to compile; `Config`, `ConfigError` and the constants are not defined.

**Step 3: Write the implementation above the test module**

```rust
//! Platform configuration written by NixOS for the `lmx` binaries.
//!
//! The LimaNix platform renders [`Config`] into [`CONFIG_PATH`] from the same declaration that builds
//! the system. A booted generation and its configuration therefore always match, and the binaries
//! never receive configuration from the host at runtime.
//!
//! Unknown fields are rejected: the file and the binary come from one generation, so a mismatch is a
//! packaging error rather than a compatibility case.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// Path of the configuration inside a booted guest.
pub const CONFIG_PATH: &str = "/etc/lmx/config.json";

/// Configuration schema understood by this crate.
pub const CONFIG_SCHEMA: u32 = 1;

/// Platform configuration of one guest generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Schema version; [`Config::from_json`] accepts only [`CONFIG_SCHEMA`].
    pub schema: u32,
    /// Identity of the virtual machine.
    pub vm: Vm,
    /// Host-assigned identifier of the generation this system was built from.
    pub generation: String,
    /// Development account.
    pub user: User,
    /// Catalog selectors chosen in `limanix.toml`, in configuration order.
    pub modules: Vec<String>,
    /// Free-space thresholds of the guest disk.
    pub disk: DiskPolicy,
    /// Named-session provider selected by catalog modules.
    pub session: Session,
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

/// Failure to obtain a usable configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("cannot read {}: {source}", path.display())]
    Read {
        /// Path that was read.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: io::Error,
    },
    /// The file is not valid configuration JSON.
    #[error("invalid configuration: {0}")]
    Invalid(#[from] serde_json::Error),
    /// The file uses a schema this binary does not understand.
    #[error("unsupported configuration schema {found}; this lmx reads schema {CONFIG_SCHEMA}")]
    Schema {
        /// Schema found in the file.
        found: u32,
    },
}

impl Config {
    /// Reads and validates the configuration at `path`.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let data = fs::read(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_json(&data)
    }

    /// Parses configuration JSON, checking its schema before its fields.
    ///
    /// Another schema usually has other fields, so it is reported as [`ConfigError::Schema`] rather
    /// than as an unknown or missing field.
    pub fn from_json(data: &[u8]) -> Result<Self, ConfigError> {
        let Header { schema } = serde_json::from_slice(data)?;
        if schema != CONFIG_SCHEMA {
            return Err(ConfigError::Schema { found: schema });
        }
        Ok(serde_json::from_slice(data)?)
    }
}

/// Schema version of a configuration file, read before its other fields.
#[derive(Deserialize)]
#[serde(expecting = "a configuration object")]
struct Header {
    /// Schema version of the file.
    schema: u32,
}
```

**Step 4: Run the tests to verify they pass**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model config
```

Expected: 5 tests pass and there are no warnings.

**Step 5: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx-model
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Add lmx platform configuration model"
```

---

## Task 3: Envelope and error codes (`lmx-model::Envelope`)

Every `--json` answer is one envelope: `contract` lets the host refuse an answer it cannot decode, and `code` stays
stable for programs.

**Files:**
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-model/src/contract.rs`
- Modify: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-model/src/lib.rs`

**Step 1: Write the failing tests**

Create `crates/lmx-model/src/contract.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_carries_data_without_error() {
        let json = serde_json::to_value(Envelope::success(7)).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({"contract": 1, "ok": true, "data": 7})
        );
    }

    #[test]
    fn failure_carries_a_stable_code() {
        let envelope = Envelope::<()>::failure(ErrorBody {
            code: ErrorCode::DiskLow,
            message: "less than 10% of the guest disk is free".into(),
            details: Map::new(),
        });
        let json = serde_json::to_value(envelope).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({
                "contract": 1,
                "ok": false,
                "error": {"code": "disk.low", "message": "less than 10% of the guest disk is free"}
            })
        );
    }

    #[test]
    fn failure_decodes_without_data() {
        /// A result without a meaningful default, like most command results.
        #[derive(Debug, PartialEq, Deserialize)]
        struct Reserve {
            /// Bytes freed by the collection.
            freed_bytes: u64,
        }

        let envelope: Envelope<Reserve> = serde_json::from_str(
            r#"{"contract": 1, "ok": false, "error": {"code": "disk.low", "message": "full"}}"#,
        )
        .expect("a failure decodes");
        assert_eq!(envelope.data, None);
        assert_eq!(
            envelope.error.map(|error| error.code),
            Some(ErrorCode::DiskLow)
        );
    }

    #[test]
    fn error_codes_keep_their_wire_names() {
        for (code, name) in [
            (ErrorCode::OwnerUnavailable, "owner.unavailable"),
            (ErrorCode::ApplyBuildFailed, "apply.build_failed"),
            (ErrorCode::ApplyCancelled, "apply.cancelled"),
            (ErrorCode::DiskLow, "disk.low"),
            (ErrorCode::NetworkUnreachable, "network.unreachable"),
            (ErrorCode::PermissionDenied, "permission.denied"),
            (ErrorCode::GenerationMismatch, "generation.mismatch"),
            (ErrorCode::ContractUnsupported, "contract.unsupported"),
        ] {
            assert_eq!(serde_json::to_value(code).expect("serialize"), name);
            assert_eq!(
                serde_json::from_value::<ErrorCode>(name.into()).expect("deserialize"),
                code
            );
        }
    }
}
```

Add to `lib.rs` below `mod config;` and its `pub use`:

```rust
mod contract;

pub use contract::{CONTRACT_VERSION, Envelope, ErrorBody, ErrorCode};
```

**Step 2: Run the tests to verify they fail**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model contract
```

Expected: FAIL to compile; `Envelope`, `ErrorBody` and `ErrorCode` are not defined.

**Step 3: Write the implementation above the test module**

````rust
//! Versioned envelope of every `--json` answer.
//!
//! The LimaNix host reads one envelope from standard output per command:
//!
//! ```text
//! {"contract": 1, "ok": true,  "data": {…}}
//! {"contract": 1, "ok": false, "error": {"code": "disk.low", "message": "…", "details": {…}}}
//! ```
//!
//! `contract` lets the host refuse an answer it cannot decode instead of misreading it. `code` is for
//! programs and stays stable; `message` is for people and may change.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Host contract version written into every envelope.
pub const CONTRACT_VERSION: u32 = 1;

/// One `--json` answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope<T> {
    /// Contract version; always [`CONTRACT_VERSION`] when written by this crate.
    pub contract: u32,
    /// Whether the command succeeded; selects `data` or `error`.
    pub ok: bool,
    /// Result of a successful command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// Reason of a failed command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl<T> Envelope<T> {
    /// Wraps the result of a successful command.
    #[must_use]
    pub fn success(data: T) -> Self {
        Self {
            contract: CONTRACT_VERSION,
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    /// Wraps the reason of a failed command.
    #[must_use]
    pub fn failure(error: ErrorBody) -> Self {
        Self {
            contract: CONTRACT_VERSION,
            ok: false,
            data: None,
            error: Some(error),
        }
    }
}

/// Reason of a failed command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// Stable machine-readable code.
    pub code: ErrorCode,
    /// Explanation for people.
    pub message: String,
    /// Code-specific values, such as disk usage for [`ErrorCode::DiskLow`].
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub details: Map<String, Value>,
}

/// Stable failure codes of the host contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// `lmxd` is not reachable.
    #[serde(rename = "owner.unavailable")]
    OwnerUnavailable,
    /// `nixos-rebuild` failed for the requested generation.
    #[serde(rename = "apply.build_failed")]
    ApplyBuildFailed,
    /// The operation was cancelled by an explicit request.
    #[serde(rename = "apply.cancelled")]
    ApplyCancelled,
    /// Free bytes or inodes are below the platform minimum.
    #[serde(rename = "disk.low")]
    DiskLow,
    /// A required network destination, such as the binary cache, is unreachable.
    #[serde(rename = "network.unreachable")]
    NetworkUnreachable,
    /// The caller is not allowed to run the operation.
    #[serde(rename = "permission.denied")]
    PermissionDenied,
    /// The mounted inputs belong to a different generation than requested.
    #[serde(rename = "generation.mismatch")]
    GenerationMismatch,
    /// The caller requested a contract version this binary does not speak.
    #[serde(rename = "contract.unsupported")]
    ContractUnsupported,
}
````

**Step 4: Run the tests to verify they pass**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model contract
```

Expected: 4 tests pass.

**Step 5: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx-model
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Add host contract envelope and error codes"
```

---

## Task 4: Status and published contract examples (`lmx-model::Status`)

The examples in `contract/v1/` are the published contract. The client in M1c tests its decoder against copies of
them, so both must decode and re-encode without loss.

**Files:**
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/contract/v1/status.json`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/contract/v1/status-partial.json`
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-model/src/status.rs`
- Modify: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-model/src/lib.rs`

**Step 1: Publish the examples**

`contract/v1/status.json` is a complete answer:

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
    "failed_units": []
  }
}
```

`contract/v1/status-partial.json` is an answer with unreadable facts:

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
    "problems": [
      {
        "fact": "interfaces",
        "message": "cannot run /run/current-system/sw/bin/ip: No such file or directory (os error 2)"
      }
    ]
  }
}
```

**Step 2: Write the failing test**

Create `crates/lmx-model/src/status.rs` with the test module:

```rust
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
```

Replace `crates/lmx-model/src/lib.rs` with its final form:

```rust
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

pub use config::{
    CONFIG_PATH, CONFIG_SCHEMA, Config, ConfigError, DiskPolicy, Session, Tools, User, Vm,
};
pub use contract::{CONTRACT_VERSION, Envelope, ErrorBody, ErrorCode};
pub use status::{DiskUsage, Generations, Interface, Problem, Status};
```

**Step 3: Run the test to verify it fails**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model status
```

Expected: FAIL to compile; `Status`, `Generations`, `DiskUsage`, `Interface` and `Problem` are not defined.

**Step 4: Write the implementation above the test module**

```rust
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
```

**Step 5: Run every model test**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model
```

Expected: 10 tests pass.

**Step 6: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add contract crates/lmx-model
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Add guest status model and contract v1 examples"
```

---

## Task 5: Fact errors, command runner and disk usage (`lmx-facts`)

**Files:**
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-facts/src/error.rs`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-facts/src/command.rs`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-facts/src/disk.rs`
- Modify: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-facts/src/lib.rs`

**Step 1: Add the shared error and the command runner**

`crates/lmx-facts/src/error.rs`:

```rust
//! Failure of one fact reader.

use std::{io, process::ExitStatus, time::Duration};

/// Failure of one fact reader.
///
/// Callers report the failure next to the other facts instead of stopping, so every variant renders
/// a complete sentence for people.
#[derive(Debug, thiserror::Error)]
pub enum FactError {
    /// A system call on a file or file system failed.
    #[error("cannot read {what}: {source}")]
    Io {
        /// What was read.
        what: &'static str,
        /// Underlying I/O failure.
        #[source]
        source: io::Error,
    },
    /// A program could not be started.
    #[error("cannot run {program}: {source}")]
    Spawn {
        /// Program path as configured.
        program: String,
        /// Underlying I/O failure.
        #[source]
        source: io::Error,
    },
    /// A program ran and reported failure.
    #[error("{program} failed ({status}){}", stderr_suffix(stderr))]
    Command {
        /// Program path as configured.
        program: String,
        /// Exit status.
        status: ExitStatus,
        /// Last part of the program's standard error.
        stderr: String,
    },
    /// A program did not finish in time.
    #[error("{program} did not finish within {timeout:?}")]
    Timeout {
        /// Program path as configured.
        program: String,
        /// How long the reader waited.
        timeout: Duration,
    },
    /// A program's output did not have the expected shape.
    #[error("unexpected {what} output: {detail}")]
    Parse {
        /// Which output was parsed.
        what: &'static str,
        /// Parser explanation.
        detail: String,
    },
}

/// Appends standard error to a message when there is any.
fn stderr_suffix(stderr: &str) -> String {
    if stderr.is_empty() {
        String::new()
    } else {
        format!(": {stderr}")
    }
}
```

`crates/lmx-facts/src/command.rs`, with tests that run `/bin/sh` as the tool:

```rust
//! Runs one system tool and returns its standard output.

use std::{
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use crate::FactError;

/// Longest standard-error tail kept in [`FactError::Command`].
const STDERR_TAIL: usize = 2048;

/// Longest wait for one tool.
///
/// `lmx status` runs `ip` and `systemctl` in turn and the host waits 10 seconds for its answer, so a
/// stuck tool, such as `systemctl` while PID 1 does not answer, still leaves time for the other facts.
const TIMEOUT: Duration = Duration::from_secs(3);

/// Runs `program` with `args` and returns standard output if it exits successfully within
/// [`TIMEOUT`].
///
/// Standard input is closed so a tool that unexpectedly prompts fails instead of waiting.
pub(crate) fn output(program: &Path, args: &[&str]) -> Result<Vec<u8>, FactError> {
    output_within(program, args, TIMEOUT)
}

/// Runs `program` like [`output`], waiting at most `timeout`.
///
/// A tool that is still running is left to finish on its own instead of being killed: `systemctl`
/// gives up on D-Bus after 25 seconds, and a tool that writes after `lmx` has exited gets `SIGPIPE`.
fn output_within(program: &Path, args: &[&str], timeout: Duration) -> Result<Vec<u8>, FactError> {
    let spawn_error = |source| FactError::Spawn {
        program: program.display().to_string(),
        source,
    };
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .spawn(move || sender.send(command.output()))
        .map_err(spawn_error)?;
    let output = receiver
        .recv_timeout(timeout)
        .map_err(|_| FactError::Timeout {
            program: program.display().to_string(),
            timeout,
        })?
        .map_err(spawn_error)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let start = stderr.len().saturating_sub(STDERR_TAIL);
        let start = (start..stderr.len())
            .find(|index| stderr.is_char_boundary(*index))
            .unwrap_or(stderr.len());
        return Err(FactError::Command {
            program: program.display().to_string(),
            status: output.status,
            stderr: stderr[start..].to_owned(),
        });
    }

    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// Runs `script` with `/bin/sh` as the tool.
    fn shell(script: &str, timeout: Duration) -> Result<Vec<u8>, FactError> {
        output_within(Path::new("/bin/sh"), &["-c", script], timeout)
    }

    #[test]
    fn returns_standard_output_of_a_successful_tool() {
        assert_eq!(shell("echo ok", TIMEOUT).expect("sh succeeds"), b"ok\n");
    }

    #[test]
    fn reports_the_exit_status_with_standard_error() {
        let error = shell("echo 'no bus' >&2; exit 3", TIMEOUT).expect_err("sh exits with 3");
        assert_eq!(error.to_string(), "/bin/sh failed (exit status: 3): no bus");
    }

    #[test]
    fn keeps_the_end_of_long_standard_error_on_a_character_boundary() {
        // 3000 bytes of three-byte characters: the last 2048 bytes start inside a character.
        let script = format!("printf %s '{}' >&2; exit 1", "€".repeat(1000));
        let error = shell(&script, TIMEOUT).expect_err("sh exits with 1");
        let FactError::Command { stderr, .. } = error else {
            panic!("not a command failure: {error}");
        };
        assert_eq!(stderr, "€".repeat(682));
    }

    #[test]
    fn reports_a_program_that_cannot_start() {
        let error = output(Path::new("/nonexistent/ip"), &[]).expect_err("no such program");
        assert_eq!(
            error.to_string(),
            "cannot run /nonexistent/ip: No such file or directory (os error 2)"
        );
    }

    #[test]
    fn stops_waiting_for_a_tool_that_hangs() {
        let started = Instant::now();
        let error = shell("exec sleep 5", Duration::from_millis(100)).expect_err("sleep hangs");
        assert!(matches!(error, FactError::Timeout { .. }), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
```

`crates/lmx-facts/src/lib.rs`. The final crate documentation comes in Task 8.

```rust
//! # lmx-facts
#![forbid(unsafe_code)]

mod command;
pub mod disk;
mod error;

pub use error::FactError;
```

**Step 2: Write the failing disk tests**

Create `crates/lmx-facts/src/disk.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_bytes_in_fragments() {
        let usage = from_blocks(Blocks {
            fragment_size: 4096,
            total: 4_000_000,
            free: 1_500_000,
            available: 1_300_000,
            inodes: 1_000_000,
            free_inodes: 600_000,
        });
        assert_eq!(usage.bytes, 16_384_000_000);
        assert_eq!(usage.free_bytes, 6_144_000_000);
        assert_eq!(usage.available_bytes, 5_324_800_000);
        assert_eq!((usage.inodes, usage.free_inodes), (1_000_000, 600_000));
    }

    #[test]
    fn reads_a_real_file_system() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let usage = usage(directory.path()).expect("statvfs on a temporary directory");
        assert!(usage.bytes > 0 && usage.free_bytes <= usage.bytes);
        assert!(usage.available_bytes <= usage.free_bytes);
    }

    #[test]
    fn explains_an_unreadable_file_system() {
        let error = usage(Path::new("/nonexistent/lmx")).expect_err("no such path");
        assert_eq!(
            error.to_string(),
            "cannot read guest disk usage: No such file or directory (os error 2)"
        );
    }
}
```

**Step 3: Run the tests to verify they fail**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts disk
```

Expected: FAIL to compile; `Blocks`, `from_blocks` and `usage` are not defined.

**Step 4: Write the implementation above the test module**

```rust
//! Usage of the file system that holds the Nix store.
//!
//! The store, the system and service data share the guest's root file system; the home directory is
//! a host mount and is not counted.

use std::path::Path;

use lmx_model::DiskUsage;

use crate::FactError;

/// Store path inside a booted guest.
pub const STORE_PATH: &str = "/nix/store";

/// Reads bytes and inodes of the file system containing `path`.
pub fn usage(path: &Path) -> Result<DiskUsage, FactError> {
    let stat = rustix::fs::statvfs(path).map_err(|error| FactError::Io {
        what: "guest disk usage",
        source: error.into(),
    })?;

    Ok(from_blocks(Blocks {
        fragment_size: stat.f_frsize,
        total: stat.f_blocks,
        free: stat.f_bfree,
        available: stat.f_bavail,
        inodes: stat.f_files,
        free_inodes: stat.f_ffree,
    }))
}

/// Raw `statvfs` counters, in fragments and inodes.
struct Blocks {
    /// Fragment size in bytes; the unit of the block counters.
    fragment_size: u64,
    /// Total fragments.
    total: u64,
    /// Free fragments, including the root reserve.
    free: u64,
    /// Fragments available to unprivileged users.
    available: u64,
    /// Total inodes.
    inodes: u64,
    /// Free inodes.
    free_inodes: u64,
}

/// Converts counters to bytes, saturating instead of overflowing.
fn from_blocks(blocks: Blocks) -> DiskUsage {
    DiskUsage {
        bytes: blocks.total.saturating_mul(blocks.fragment_size),
        free_bytes: blocks.free.saturating_mul(blocks.fragment_size),
        available_bytes: blocks.available.saturating_mul(blocks.fragment_size),
        inodes: blocks.inodes,
        free_inodes: blocks.free_inodes,
    }
}
```

**Step 5: Run the tests to verify they pass**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts
```

Expected: 8 tests pass, 5 for the command runner and 3 for disk usage. The library build warns that `STDERR_TAIL`,
`TIMEOUT`, `output` and `output_within` are never used: until Task 7, only the runner's own tests call it. These four
warnings are expected.

**Step 6: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx-facts
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Add guest disk usage reader"
```

---

## Task 6: Generation markers (`lmx-facts::generations`)

The desired, built and booted generations are read from three JSON markers. A missing or malformed marker is an
answer (`None`), not a failure: a system built before `lmx` existed has no markers. A marker that exists but cannot
be read is also `None`, and `read` returns its error so that the caller reports a problem. In the guest,
`/mnt/limanix` is `drwx------ limanix-admin`, so when a person runs `lmx status`, `desired` must be reported as
unreadable rather than absent.

**Files:**
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-facts/src/generations.rs`
- Modify: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-facts/src/lib.rs` (add `pub mod generations;` after `mod error;`)

**Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().expect("marker has a parent")).expect("create parent");
        fs::write(path, content).expect("write marker");
    }

    #[test]
    fn reads_each_stage_independently() {
        let root = tempfile::tempdir().expect("temporary root");
        let paths = GenerationPaths::under(root.path());
        write(
            &paths.desired,
            r#"{"name": "dev-box", "generation": "0123456789ab"}"#,
        );
        write(
            &paths.built,
            r#"{"schema": 1, "generation": "0123456789ab"}"#,
        );

        let (generations, unreadable) = read(&paths);
        assert_eq!(generations.desired.as_deref(), Some("0123456789ab"));
        assert_eq!(generations.built.as_deref(), Some("0123456789ab"));
        assert_eq!(
            generations.booted, None,
            "a system without lmx has no booted marker"
        );
        assert!(unreadable.is_empty(), "{unreadable:?}");
    }

    #[test]
    fn ignores_malformed_markers() {
        let root = tempfile::tempdir().expect("temporary root");
        let paths = GenerationPaths::under(root.path());
        write(&paths.desired, "not json");
        write(&paths.built, r#"{"generation": "../../etc"}"#);
        write(&paths.booted, r#"{"generation": ""}"#);

        let (generations, unreadable) = read(&paths);
        assert_eq!(generations, Generations::default());
        assert!(unreadable.is_empty(), "{unreadable:?}");
    }

    #[test]
    fn accepts_only_short_lowercase_identifiers() {
        let root = tempfile::tempdir().expect("temporary root");
        let paths = GenerationPaths::under(root.path());
        let longest = "a".repeat(MAX_LENGTH);
        write(&paths.desired, &format!(r#"{{"generation": "{longest}"}}"#));
        write(&paths.built, &format!(r#"{{"generation": "{longest}a"}}"#));
        write(&paths.booted, r#"{"generation": "0123456789AB"}"#);

        let (generations, _) = read(&paths);
        assert_eq!(generations.desired, Some(longest));
        assert_eq!((generations.built, generations.booted), (None, None));
    }

    #[test]
    fn reports_a_marker_that_cannot_be_read() {
        let root = tempfile::tempdir().expect("temporary root");
        let paths = GenerationPaths::under(root.path());
        // Root reads any file, so a directory stands in for the host mount that is closed to users.
        fs::create_dir_all(&paths.desired).expect("create directory");

        let (generations, unreadable) = read(&paths);
        assert_eq!(generations, Generations::default());
        let messages: Vec<String> = unreadable.iter().map(ToString::to_string).collect();
        assert_eq!(
            messages,
            ["cannot read the desired generation: Is a directory (os error 21)"]
        );
    }
}
```

**Step 2: Run the tests to verify they fail**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts generations
```

Expected: FAIL to compile; `GenerationPaths`, `read` and `MAX_LENGTH` are not defined.

**Step 3: Write the implementation above the test module**

```rust
//! Generation identifiers of the desired, built and booted systems.
//!
//! Each stage keeps a JSON file with a top-level `generation` string:
//!
//! | Stage   | Marker                                             | Written by               |
//! |---------|----------------------------------------------------|--------------------------|
//! | desired | `/mnt/limanix/flake/runtime.json`                  | the host, per generation |
//! | built   | `/nix/var/nix/profiles/system/etc/lmx/config.json` | NixOS, at build          |
//! | booted  | `/run/booted-system/etc/lmx/config.json`           | NixOS, at build          |
//!
//! A missing or malformed marker gives `None`: a system built before `lmx` existed has no marker,
//! and that is an answer rather than a failure. A marker that exists but cannot be read also gives
//! `None`, and the failure is returned with it: the host mount is closed to people in the guest, so
//! their answer is partial and must be marked as such.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use lmx_model::Generations;
use serde::Deserialize;

use crate::FactError;

/// Longest accepted generation identifier.
const MAX_LENGTH: usize = 64;

/// Locations of the three generation markers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationPaths {
    /// Marker of the generation mounted by the host.
    pub desired: PathBuf,
    /// Marker of the current system profile.
    pub built: PathBuf,
    /// Marker of the running system.
    pub booted: PathBuf,
}

impl GenerationPaths {
    /// Marker locations inside a booted guest.
    pub fn system() -> Self {
        Self::under(Path::new("/"))
    }

    /// Marker locations below `root`, for tests and offline inspection.
    pub fn under(root: &Path) -> Self {
        Self {
            desired: root.join("mnt/limanix/flake/runtime.json"),
            built: root.join("nix/var/nix/profiles/system/etc/lmx/config.json"),
            booted: root.join("run/booted-system/etc/lmx/config.json"),
        }
    }
}

/// Reads all three markers, with the failures of markers that exist but cannot be read.
pub fn read(paths: &GenerationPaths) -> (Generations, Vec<FactError>) {
    let mut unreadable = Vec::new();
    let mut stage = |what, path| {
        marker(what, path).unwrap_or_else(|error| {
            unreadable.push(error);
            None
        })
    };
    let generations = Generations {
        desired: stage("the desired generation", &paths.desired),
        built: stage("the built generation", &paths.built),
        booted: stage("the booted generation", &paths.booted),
    };
    (generations, unreadable)
}

/// The only field read from a marker; other fields belong to their own schemas.
#[derive(Deserialize)]
struct Marker {
    /// Generation identifier.
    generation: String,
}

/// Reads one marker, accepting only a short identifier of lowercase letters and digits.
fn marker(what: &'static str, path: &Path) -> Result<Option<String>, FactError> {
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(FactError::Io { what, source }),
    };
    let Ok(Marker { generation }) = serde_json::from_slice(&data) else {
        return Ok(None);
    };
    let valid = !generation.is_empty()
        && generation.len() <= MAX_LENGTH
        && generation
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
    Ok(valid.then_some(generation))
}
```

**Step 4: Run the tests to verify they pass**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts generations
```

Expected: 4 tests pass.

**Step 5: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx-facts
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Read desired, built and booted generations"
```

---

## Task 7: Network interfaces (`lmx-facts::network`)

The guest reports every interface with its hardware address. The host then picks the address of the VM's shared
network, as `client/internal/guest/address.go` does today.

**Files:**
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-facts/src/network.rs`
- Modify: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-facts/src/lib.rs` (add `pub mod network;`)

**Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_global_ipv4_with_hardware_addresses() {
        let output = br#"[
            {"ifname": "lo", "address": "00:00:00:00:00:00",
             "addr_info": [{"family": "inet", "scope": "host", "local": "127.0.0.1"}]},
            {"ifname": "enp0s1", "address": "52:55:55:AA:BB:CC",
             "addr_info": [
                {"family": "inet", "scope": "global", "local": "192.0.2.10"},
                {"family": "inet6", "scope": "global", "local": "2001:db8::10"},
                {"family": "inet", "scope": "link", "local": "169.254.1.2"}
             ]},
            {"ifname": "wg0", "addr_info": [{"family": "inet", "scope": "global", "local": "10.0.0.2"}]}
        ]"#;

        let interfaces = parse(output).expect("valid ip output");
        assert_eq!(
            interfaces,
            vec![
                Interface {
                    name: "lo".into(),
                    mac: Some("00:00:00:00:00:00".into()),
                    ipv4: vec![]
                },
                Interface {
                    name: "enp0s1".into(),
                    mac: Some("52:55:55:aa:bb:cc".into()),
                    ipv4: vec!["192.0.2.10".into()],
                },
                Interface {
                    name: "wg0".into(),
                    mac: None,
                    ipv4: vec!["10.0.0.2".into()]
                },
            ]
        );
    }

    #[test]
    fn reports_no_hardware_address_for_ip_tunnels() {
        let output = br#"[
            {"ifname": "tunl0", "link_type": "ipip", "address": "0.0.0.0"},
            {"ifname": "ip6tnl0", "link_type": "tunnel6", "address": "::"}
        ]"#;

        let interfaces = parse(output).expect("valid ip output");
        assert_eq!(interfaces.len(), 2);
        assert!(
            interfaces.iter().all(|interface| interface.mac.is_none()),
            "{interfaces:?}"
        );
    }

    #[test]
    fn rejects_other_output() {
        let error = parse(b"Device \"eth0\" does not exist.").expect_err("not JSON");
        assert!(
            error
                .to_string()
                .starts_with("unexpected ip address output"),
            "{error}"
        );
    }
}
```

**Step 2: Run the tests to verify they fail**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts network
```

Expected: FAIL to compile; `parse` is not defined.

**Step 3: Write the implementation above the test module**

```rust
//! Network interfaces and their global IPv4 addresses.
//!
//! The host selects the address it shows in `limanix list` by matching hardware addresses with the
//! VM's shared network. The guest therefore reports every interface with its hardware address and
//! leaves the choice to the host.

use std::{net::Ipv4Addr, path::Path};

use lmx_model::Interface;
use serde::Deserialize;

use crate::{FactError, command};

/// Reads interfaces with `ip -j address show`.
pub fn interfaces(ip: &Path) -> Result<Vec<Interface>, FactError> {
    let output = command::output(ip, &["-j", "address", "show"])?;
    parse(&output)
}

/// One link in `ip -j address show` output.
#[derive(Deserialize)]
struct Link {
    /// Kernel interface name.
    ifname: String,
    /// Hardware address, or the endpoint of an IP tunnel; absent for some virtual links.
    #[serde(default)]
    address: Option<String>,
    /// Protocol addresses.
    #[serde(default)]
    addr_info: Vec<Address>,
}

/// One protocol address of a link.
#[derive(Deserialize)]
struct Address {
    /// `inet` or `inet6`.
    family: String,
    /// `global`, `link`, `host` and so on.
    #[serde(default)]
    scope: String,
    /// Address without prefix length.
    #[serde(default)]
    local: String,
}

/// Parses `ip -j address show` output, keeping global-scope IPv4 addresses only.
pub fn parse(output: &[u8]) -> Result<Vec<Interface>, FactError> {
    let links: Vec<Link> = serde_json::from_slice(output).map_err(|error| FactError::Parse {
        what: "ip address",
        detail: error.to_string(),
    })?;

    Ok(links
        .into_iter()
        .map(|link| Interface {
            name: link.ifname,
            mac: link
                .address
                .filter(|address| is_hardware_address(address))
                .map(|mac| mac.to_ascii_lowercase()),
            ipv4: link
                .addr_info
                .into_iter()
                .filter(|address| address.family == "inet" && address.scope == "global")
                .filter_map(|address| address.local.parse::<Ipv4Addr>().ok())
                .map(|address| address.to_string())
                .collect(),
        })
        .collect())
}

/// Whether `address` is a hardware address: two-digit hexadecimal octets separated by colons.
///
/// IP tunnels such as `ipip` and `sit` report their local endpoint in the same field, such as
/// `0.0.0.0`.
fn is_hardware_address(address: &str) -> bool {
    address
        .split(':')
        .all(|octet| octet.len() == 2 && octet.bytes().all(|byte| byte.is_ascii_hexdigit()))
}
```

**Step 4: Run the tests to verify they pass**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts network
```

Expected: 3 tests pass. `interfaces` is the first caller of the runner outside its tests, so the dead-code warnings
from Task 5 are gone.

**Step 5: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx-facts
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Read network interfaces with ip -j"
```

---

## Task 8: Failed units (`lmx-facts::units`) and crate documentation

**Files:**
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-facts/src/units.rs`
- Modify: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx-facts/src/lib.rs`

**Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_first_column() {
        let output = "limanix-store-guard.service loaded failed failed Keep free space\n\
                      docker.socket loaded failed failed Docker Socket for the API\n\n";
        assert_eq!(
            parse(output),
            ["limanix-store-guard.service", "docker.socket"]
        );
    }

    #[test]
    fn empty_output_means_no_failures() {
        assert!(parse("").is_empty());
    }
}
```

Replace `crates/lmx-facts/src/lib.rs` with its final form:

```rust
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
//! | [`network`]     | interfaces and global IPv4 addresses    | `ip -j address show`                    |
//! | [`units`]       | failed systemd units                    | `systemctl list-units --state=failed`   |
//!
//! Readers that run a program take its absolute path from the platform configuration and split
//! process I/O from a pure parser, so the parsers are tested with fixed output.
#![forbid(unsafe_code)]

mod command;
pub mod disk;
mod error;
pub mod generations;
pub mod network;
pub mod units;

pub use error::FactError;
```

**Step 2: Run the tests to verify they fail**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts units
```

Expected: FAIL to compile; `parse` is not defined in `units`.

**Step 3: Write the implementation above the test module**

```rust
//! Failed systemd units.

use std::path::Path;

use crate::{FactError, command};

/// Lists failed units with `systemctl list-units --state=failed`.
pub fn failed(systemctl: &Path) -> Result<Vec<String>, FactError> {
    let output = command::output(
        systemctl,
        &[
            "list-units",
            "--state=failed",
            "--plain",
            "--no-legend",
            "--no-pager",
        ],
    )?;
    Ok(parse(&String::from_utf8_lossy(&output)))
}

/// Parses plain `systemctl list-units` output: the unit name is the first column of each line.
pub fn parse(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}
```

**Step 4: Run every fact test**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts
```

Expected: 17 tests pass with no warnings.

**Step 5: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx-facts
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Read failed systemd units"
```

---

## Task 9: Command line and `lmx version`

**Files:**
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx/src/cli.rs`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx/src/output.rs`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx/src/version.rs`
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx/tests/cli.rs`
- Modify: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx/src/main.rs`

**Step 1: Write the failing integration tests**

`crates/lmx/tests/cli.rs` runs the built binary:

```rust
//! Command-line contract of the `lmx` binary.

use std::process::{Command, Output};

use serde_json::{Value, json};

/// Runs `lmx` with `args`.
fn lmx(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lmx"))
        .args(args)
        .output()
        .expect("run lmx")
}

/// Parses standard output as one JSON answer.
fn answer(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "lmx failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("standard output is one JSON answer")
}

#[test]
fn version_json_names_the_contract() {
    let answer = answer(&lmx(&["version", "--json"]));
    assert_eq!(
        answer,
        json!({"contract": 1, "ok": true, "data": {"version": env!("CARGO_PKG_VERSION"), "contract": 1}})
    );
}

#[test]
fn unknown_commands_are_usage_errors() {
    let output = lmx(&["frobnicate"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Usage: lmx"));
}

#[test]
fn no_command_prints_help() {
    let output = lmx(&[]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage: lmx"));
}
```

**Step 2: Run the tests to verify they fail**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx --test cli
```

Expected: 3 tests FAIL. The placeholder binary prints nothing and exits 0.

**Step 3: Write the command line**

`crates/lmx/src/cli.rs`:

```rust
//! Command-line interface.

use std::{io, process::ExitCode};

use clap::{Args, CommandFactory, Parser, Subcommand};

/// Guest owner command of a LimaNix VM.
#[derive(Debug, Parser)]
#[command(name = "lmx", version, disable_help_subcommand = true)]
pub(crate) struct Cli {
    /// Command to run; without one, `lmx` prints help.
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

/// Commands of `lmx`.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Show the lmx version and the host contract it speaks.
    Version(OutputArgs),
}

/// Output selection shared by commands that answer the host.
#[derive(Debug, Args)]
pub(crate) struct OutputArgs {
    /// Answer with the JSON host contract instead of text.
    #[arg(long)]
    pub(crate) json: bool,
}

/// Prints the generated help when `lmx` runs without a command.
pub(crate) fn print_help() -> io::Result<ExitCode> {
    Cli::command().print_help()?;
    Ok(ExitCode::SUCCESS)
}
```

`crates/lmx/src/output.rs`:

```rust
//! Exit codes and answers on standard output: JSON for the host, text for people.

use std::io::{self, Write};

use lmx_model::Envelope;
use serde::Serialize;

/// Exit status of a failed operation; details are in the JSON answer or on standard error.
pub(crate) const FAILURE: u8 = 1;

/// Writes one envelope as a single JSON line to standard output.
pub(crate) fn write_json<T: Serialize>(envelope: &Envelope<T>) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, envelope)?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}

/// Writes text for people to standard output, returning write failures instead of panicking
/// like `print!`.
pub(crate) fn write_text(text: &str) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(text.as_bytes())?;
    stdout.flush()
}
```

`crates/lmx/src/version.rs`:

```rust
//! `lmx version`: the binary and the host contract it speaks.

use std::{io, process::ExitCode};

use lmx_model::{CONTRACT_VERSION, Envelope};
use serde::Serialize;

use crate::{cli::OutputArgs, output};

/// Version answer of the host contract.
#[derive(Debug, Serialize)]
pub(crate) struct Version {
    /// Release version of the binary.
    version: &'static str,
    /// Host contract version.
    contract: u32,
}

/// Runs `lmx version`.
pub(crate) fn run(args: &OutputArgs) -> io::Result<ExitCode> {
    let version = Version {
        version: env!("CARGO_PKG_VERSION"),
        contract: CONTRACT_VERSION,
    };
    if args.json {
        output::write_json(&Envelope::success(version))?;
    } else {
        output::write_text(&format!(
            "lmx {} (host contract {})\n",
            version.version, version.contract
        ))?;
    }
    Ok(ExitCode::SUCCESS)
}
```

`crates/lmx/src/main.rs`:

```rust
//! # lmx
//!
//! Command of the LimaNix guest owner. People run it inside the VM; the LimaNix host runs it over
//! management SSH with `--json` and reads the [host contract](https://github.com/limanix/lmx/blob/main/docs/contract.md).
#![forbid(unsafe_code)]

mod cli;
mod output;
mod version;

use std::{io, process::ExitCode};

use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Command::Version(args)) => version::run(&args),
        None => cli::print_help(),
    };

    match result {
        Ok(code) => code,
        // Standard output was closed by its reader, as in `lmx version | true`: nobody is left to
        // read an answer or an error. clap ends `--help` the same way.
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lmx: {error}");
            ExitCode::from(output::FAILURE)
        }
    }
}
```

**Step 4: Run the tests to verify they pass**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx
```

Expected: 3 integration tests pass with no warnings.

**Step 5: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Add lmx command line with version"
```

---

## Task 10: `lmx status`

Each fact is read independently. A failure becomes a `Problem` next to the other facts, never an error exit, so the
host always gets what can be read.

The tests point the binary at a prepared tree through `LMX_SYSTEM_ROOT` and `LMX_CONFIG`. They also set an empty
`PATH`, so fake tools must use shell built-ins only.

**Files:**
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx/src/format.rs`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx/src/system.rs`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx/src/status.rs`
- Modify: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx/src/cli.rs`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx/src/main.rs`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/crates/lmx/tests/cli.rs`

**Step 1: Write the failing tests**

Replace `crates/lmx/tests/cli.rs` with the guest-tree version:

```rust
//! Command-line contract of the `lmx` binary, run against a prepared guest tree.

use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::{Value, json};

/// A guest tree with generation markers, a store directory, fake tools and a configuration.
struct Guest {
    /// Root replacing `/`; removed when the test ends.
    root: tempfile::TempDir,
}

impl Guest {
    /// Prepares a guest that built generation `0123456789ab` and still runs `ba9876543210`.
    fn new() -> Self {
        let guest = Self {
            root: tempfile::tempdir().expect("temporary guest root"),
        };
        guest.write("nix/store/.keep", "");
        guest.write(
            "mnt/limanix/flake/runtime.json",
            r#"{"generation": "0123456789ab"}"#,
        );
        guest.write(
            "nix/var/nix/profiles/system/etc/lmx/config.json",
            r#"{"generation": "0123456789ab"}"#,
        );
        guest.write(
            "run/booted-system/etc/lmx/config.json",
            r#"{"generation": "ba9876543210"}"#,
        );
        let ip = guest.tool(
            "ip",
            r#"[{"ifname":"enp0s1","address":"52:55:55:aa:bb:cc","addr_info":[{"family":"inet","scope":"global","local":"192.0.2.10"}]}]"#,
        );
        let systemctl = guest.tool(
            "systemctl",
            "limanix-store-guard.service loaded failed failed Guard",
        );
        let config = json!({
            "schema": 1,
            "vm": {"name": "dev-box", "arch": "arm64", "system": "NixOS 26.05"},
            "generation": "ba9876543210",
            "user": {"name": "dev", "home": "/home/dev", "uid": 501},
            "modules": [],
            "disk": {"collect_percent": 20, "minimum_percent": 10},
            "session": {"command": null, "providers": []},
            "tools": {"ip": ip, "systemctl": systemctl}
        });
        guest.write("etc/lmx/config.json", &config.to_string());
        guest
    }

    /// Writes `content` to `relative` below the root.
    fn write(&self, relative: &str, content: &str) {
        let path = self.root.path().join(relative);
        fs::create_dir_all(path.parent().expect("file has a parent")).expect("create parent");
        fs::write(path, content).expect("write file");
    }

    /// Creates an executable that prints `output` and returns its path.
    ///
    /// The script uses only shell built-ins because the tests run `lmx` with an empty `PATH`.
    fn tool(&self, name: &str, output: &str) -> PathBuf {
        assert!(!output.contains('\''), "tool output is single-quoted");
        let path = self.root.path().join("tools").join(name);
        self.write(
            &format!("tools/{name}"),
            &format!("#!/bin/sh\nprintf '%s\\n' '{output}'\n"),
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("make tool executable");
        path
    }

    /// Path of the configuration inside the tree.
    fn config(&self) -> PathBuf {
        self.root.path().join("etc/lmx/config.json")
    }

    /// Prepares `lmx` with the tree as its system root.
    fn command(&self, args: &[&str], config: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lmx"));
        command
            .args(args)
            .env("LMX_SYSTEM_ROOT", self.root.path())
            .env("LMX_CONFIG", config)
            .env("PATH", self.root.path().join("empty"));
        command
    }

    /// Runs `lmx` with the tree as its system root.
    fn lmx(&self, args: &[&str], config: &Path) -> Output {
        self.command(args, config).output().expect("run lmx")
    }
}

/// Parses standard output as one JSON answer.
fn answer(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "lmx failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let line = output
        .stdout
        .strip_suffix(b"\n")
        .expect("the answer ends with a newline");
    assert!(!line.contains(&b'\n'), "the answer is a single line");
    serde_json::from_slice(line).expect("standard output is one JSON answer")
}

#[test]
fn status_json_answers_the_host_contract() {
    let guest = Guest::new();
    let answer = answer(&guest.lmx(&["status", "--json"], &guest.config()));

    assert_eq!(answer["contract"], 1);
    assert_eq!(answer["ok"], true);
    let data = &answer["data"];
    assert_eq!(
        data["generations"],
        json!({"desired": "0123456789ab", "built": "0123456789ab", "booted": "ba9876543210"})
    );
    assert!(
        data["disk"]["bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes > 0)
    );
    assert_eq!(
        data["interfaces"],
        json!([{"name": "enp0s1", "mac": "52:55:55:aa:bb:cc", "ipv4": ["192.0.2.10"]}])
    );
    assert_eq!(data["failed_units"], json!(["limanix-store-guard.service"]));
    assert!(
        data.get("problems").is_none(),
        "every fact was read: {data}"
    );
}

#[test]
fn status_reports_missing_facts_without_failing() {
    let guest = Guest::new();
    let marker = guest.root.path().join("mnt/limanix/flake/runtime.json");
    fs::remove_file(&marker).expect("remove the desired marker");
    fs::create_dir(&marker).expect("a directory is a marker that cannot be read");
    let missing = guest.root.path().join("missing.json");
    let answer = answer(&guest.lmx(&["status", "--json"], &missing));

    let data = &answer["data"];
    assert_eq!(answer["ok"], true);
    assert_eq!(data["generations"]["desired"], Value::Null);
    assert!(
        data["disk"].is_object(),
        "disk does not depend on the configuration"
    );
    assert_eq!(data["interfaces"], Value::Null);
    assert_eq!(data["failed_units"], Value::Null);
    let facts: Vec<&str> = data["problems"]
        .as_array()
        .expect("problems are listed")
        .iter()
        .map(|problem| problem["fact"].as_str().expect("fact name"))
        .collect();
    assert_eq!(
        facts,
        ["config", "generations", "interfaces", "failed_units"]
    );
}

#[test]
fn status_text_is_for_people() {
    let guest = Guest::new();
    let output = guest.lmx(&["status"], &guest.config());
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("UTF-8 text");
    assert!(
        text.contains("built 0123456789ab, booted ba9876543210"),
        "{text}"
    );
    assert!(text.contains("Network       enp0s1 192.0.2.10"), "{text}");
    assert!(
        text.contains("Failed units  limanix-store-guard.service"),
        "{text}"
    );
}

#[test]
fn version_json_names_the_contract() {
    let guest = Guest::new();
    let answer = answer(&guest.lmx(&["version", "--json"], &guest.config()));
    assert_eq!(
        answer,
        json!({"contract": 1, "ok": true, "data": {"version": env!("CARGO_PKG_VERSION"), "contract": 1}})
    );
}

#[test]
fn unknown_commands_are_usage_errors() {
    let guest = Guest::new();
    let output = guest.lmx(&["frobnicate"], &guest.config());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Usage: lmx"));
}

#[test]
fn no_command_prints_help() {
    let guest = Guest::new();
    let output = guest.lmx(&[], &guest.config());
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage: lmx"));
}

#[test]
fn a_closed_standard_output_ends_quietly() {
    let guest = Guest::new();
    for args in [&["status"][..], &["status", "--json"], &["version"], &[]] {
        let (reader, writer) = io::pipe().expect("pipe");
        drop(reader);
        let output = guest
            .command(args, &guest.config())
            .stdout(writer)
            .output()
            .expect("run lmx");
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert!(
            output.stderr.is_empty(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
```

Create `crates/lmx/src/format.rs` with its tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_to_tenths_below_ten_gibibytes() {
        assert_eq!(gibibytes(0), "0.0 GiB");
        assert_eq!(gibibytes(GIB / 2), "0.5 GiB");
        assert_eq!(gibibytes(9 * GIB + GIB / 4), "9.3 GiB");
    }

    #[test]
    fn rounds_to_whole_gibibytes_from_ten() {
        assert_eq!(gibibytes(10 * GIB), "10 GiB");
        assert_eq!(gibibytes(38 * GIB + GIB / 2), "39 GiB");
        assert_eq!(gibibytes(10 * GIB + GIB / 2 - 1), "10 GiB");
    }
}
```

Create `crates/lmx/src/status.rs` with its test module:

```rust
#[cfg(test)]
mod tests {
    use lmx_model::{DiskUsage, Generations, Interface};

    use super::*;

    #[test]
    fn renders_every_fact_on_its_own_line() {
        let status = Status {
            generations: Generations {
                desired: Some("0123456789ab".into()),
                built: Some("0123456789ab".into()),
                booted: None,
            },
            disk: Some(DiskUsage {
                bytes: 16 << 30,
                free_bytes: 9 << 30,
                available_bytes: 8 << 30,
                inodes: 1_048_576,
                free_inodes: 495_616,
            }),
            interfaces: Some(vec![
                Interface {
                    name: "lo".into(),
                    mac: None,
                    ipv4: vec![],
                },
                Interface {
                    name: "enp0s1".into(),
                    mac: None,
                    ipv4: vec!["192.0.2.10".into()],
                },
            ]),
            failed_units: Some(vec![]),
            problems: vec![],
        };
        assert_eq!(
            render(&status),
            "Generation    desired 0123456789ab, built 0123456789ab, booted unknown\n\
             Disk          9.0 GiB of 16 GiB free, 495616 of 1048576 inodes free\n\
             Network       enp0s1 192.0.2.10\n\
             Failed units  none\n"
        );
    }

    #[test]
    fn keeps_continuation_lines_of_a_problem_in_the_value_column() {
        let status = Status {
            problems: vec![Problem {
                fact: "interfaces".into(),
                message: "ip failed (exit status: 1): invalid option\nUsage: ip OBJECT".into(),
            }],
            ..Status::default()
        };
        let text = render(&status);
        let problem: Vec<&str> = text.lines().skip(4).collect();
        assert_eq!(
            problem,
            [
                "Problem       interfaces: ip failed (exit status: 1): invalid option",
                "              Usage: ip OBJECT",
            ]
        );
    }
}
```

Replace the module list in `crates/lmx/src/main.rs` with this one, so Cargo compiles the new tests:

```rust
mod cli;
mod format;
mod output;
mod status;
mod version;
```

**Step 2: Run the tests to verify they fail**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx
```

Expected: FAIL to compile; `gibibytes`, `GIB`, `render`, `Status` and `Problem` are not defined.

**Step 3: Implement formatting, system locations and status**

`format.rs`, above its tests:

```rust
//! Value formatting shared by text output.

/// Bytes in one gibibyte.
const GIB: u64 = 1 << 30;

/// Formats bytes as gibibytes: one decimal below 10 GiB, whole numbers from 10 GiB.
pub(crate) fn gibibytes(bytes: u64) -> String {
    let (bytes, gib) = (u128::from(bytes), u128::from(GIB));
    let tenths = (bytes * 10 + gib / 2) / gib;
    if tenths >= 100 {
        // Rounded from bytes, not from tenths: 10.47 GiB is 10, not 10.5 rounded up to 11.
        format!("{} GiB", (bytes + gib / 2) / gib)
    } else {
        format!("{}.{} GiB", tenths / 10, tenths % 10)
    }
}
```

`crates/lmx/src/system.rs`:

```rust
//! Locations and configuration of the running guest.
//!
//! Without a readable configuration, `ip` and `systemctl` are looked up in `PATH`, so `lmx status`
//! still reports interfaces and failed units; its `config` problem marks the answer as degraded.

use std::{
    env,
    path::{Path, PathBuf},
};

use lmx_facts::{disk::STORE_PATH, generations::GenerationPaths};
use lmx_model::{CONFIG_PATH, Config};

/// Guest locations and the platform configuration, resolved once per command.
#[derive(Debug)]
pub(crate) struct System {
    /// Prefix of system paths; `/` outside tests.
    root: PathBuf,
    /// Platform configuration, or why it could not be read.
    pub(crate) config: Result<Config, String>,
}

impl System {
    /// Resolves locations from the environment, honoring the test hooks.
    pub(crate) fn from_environment() -> Self {
        let root = hook("LMX_SYSTEM_ROOT").unwrap_or_else(|| PathBuf::from("/"));
        let config_path = hook("LMX_CONFIG").unwrap_or_else(|| PathBuf::from(CONFIG_PATH));
        Self::new(root, &config_path)
    }

    /// Resolves locations below `root` and reads the configuration at `config_path`.
    pub(crate) fn new(root: PathBuf, config_path: &Path) -> Self {
        let config = Config::load(config_path).map_err(|error| error.to_string());
        Self { root, config }
    }

    /// Path whose file system holds the Nix store.
    pub(crate) fn store(&self) -> PathBuf {
        self.root.join(STORE_PATH.trim_start_matches('/'))
    }

    /// Locations of the generation markers.
    pub(crate) fn generation_paths(&self) -> GenerationPaths {
        GenerationPaths::under(&self.root)
    }

    /// `ip` from the configuration, or from `PATH` when the configuration is unreadable.
    pub(crate) fn ip(&self) -> PathBuf {
        self.config.as_ref().map_or_else(
            |_| PathBuf::from("ip"),
            |config| PathBuf::from(&config.tools.ip),
        )
    }

    /// `systemctl` from the configuration, or from `PATH` when the configuration is unreadable.
    pub(crate) fn systemctl(&self) -> PathBuf {
        self.config.as_ref().map_or_else(
            |_| PathBuf::from("systemctl"),
            |config| PathBuf::from(&config.tools.systemctl),
        )
    }
}

/// Path from a test hook, or `None` when the variable is unset or empty.
fn hook(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}
```

`status.rs`, above its tests:

```rust
//! `lmx status`: what the guest is right now.
//!
//! Every fact is read independently. A fact that cannot be read becomes `null` with a
//! [`Problem`], so the host and people always get the rest.

use std::{io, process::ExitCode};

use lmx_facts::{FactError, disk, generations, network, units};
use lmx_model::{Envelope, Problem, Status};

use crate::{cli::OutputArgs, format::gibibytes, output, system::System};

/// Runs `lmx status`.
pub(crate) fn run(system: &System, args: &OutputArgs) -> io::Result<ExitCode> {
    let status = collect(system);
    if args.json {
        output::write_json(&Envelope::success(status))?;
    } else {
        output::write_text(&render(&status))?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Reads every fact of [`Status`].
pub(crate) fn collect(system: &System) -> Status {
    let mut problems = Vec::new();
    if let Err(message) = &system.config {
        problems.push(Problem {
            fact: "config".into(),
            message: message.clone(),
        });
    }

    let (generations, unreadable) = generations::read(&system.generation_paths());
    problems.extend(unreadable.into_iter().map(|error| Problem {
        fact: "generations".into(),
        message: error.to_string(),
    }));

    Status {
        generations,
        disk: record("disk", disk::usage(&system.store()), &mut problems),
        interfaces: record(
            "interfaces",
            network::interfaces(&system.ip()),
            &mut problems,
        ),
        failed_units: record(
            "failed_units",
            units::failed(&system.systemctl()),
            &mut problems,
        ),
        problems,
    }
}

/// Keeps a fact, or records why it is missing.
fn record<T>(fact: &str, result: Result<T, FactError>, problems: &mut Vec<Problem>) -> Option<T> {
    result
        .map_err(|error| {
            problems.push(Problem {
                fact: fact.into(),
                message: error.to_string(),
            });
        })
        .ok()
}

/// Width of the label column in text output.
const LABEL: usize = 14;

/// Renders status as aligned text for people.
pub(crate) fn render(status: &Status) -> String {
    let unknown = || "unknown".to_owned();
    let generations = &status.generations;
    let mut text = String::new();
    // Continuation lines, such as a tool's multi-line standard error, stay in the value column.
    let mut row = |label: &str, value: String| {
        let mut lines = value.lines();
        let first = lines.next().unwrap_or_default();
        text.push_str(&format!("{label:<LABEL$}{first}\n"));
        for line in lines {
            text.push_str(&format!("{:LABEL$}{line}\n", ""));
        }
    };

    row(
        "Generation",
        format!(
            "desired {}, built {}, booted {}",
            generations.desired.clone().unwrap_or_else(unknown),
            generations.built.clone().unwrap_or_else(unknown),
            generations.booted.clone().unwrap_or_else(unknown),
        ),
    );
    row(
        "Disk",
        status.disk.map_or_else(unknown, |disk| {
            format!(
                "{} of {} free, {} of {} inodes free",
                gibibytes(disk.free_bytes),
                gibibytes(disk.bytes),
                disk.free_inodes,
                disk.inodes
            )
        }),
    );
    row(
        "Network",
        status
            .interfaces
            .as_ref()
            .map_or_else(unknown, |interfaces| {
                let addressed: Vec<String> = interfaces
                    .iter()
                    .filter(|interface| !interface.ipv4.is_empty())
                    .map(|interface| format!("{} {}", interface.name, interface.ipv4.join(" ")))
                    .collect();
                if addressed.is_empty() {
                    "no global IPv4 address".into()
                } else {
                    addressed.join(", ")
                }
            }),
    );
    row(
        "Failed units",
        status.failed_units.as_ref().map_or_else(unknown, |units| {
            if units.is_empty() {
                "none".into()
            } else {
                units.join(", ")
            }
        }),
    );
    for problem in &status.problems {
        row("Problem", format!("{}: {}", problem.fact, problem.message));
    }
    text
}
```

Replace `crates/lmx/src/cli.rs`:

```rust
//! Command-line interface.

use std::{io, process::ExitCode};

use clap::{Args, CommandFactory, Parser, Subcommand};

/// Guest owner command of a LimaNix VM.
#[derive(Debug, Parser)]
#[command(name = "lmx", version, disable_help_subcommand = true)]
pub(crate) struct Cli {
    /// Command to run; without one, `lmx` prints help.
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

/// Commands of `lmx`.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Show generations, disk usage, network interfaces and failed units.
    Status(OutputArgs),
    /// Show the lmx version and the host contract it speaks.
    Version(OutputArgs),
}

/// Output selection shared by commands that answer the host.
#[derive(Debug, Args)]
pub(crate) struct OutputArgs {
    /// Answer with the JSON host contract instead of text.
    #[arg(long)]
    pub(crate) json: bool,
}

/// Prints the generated help when `lmx` runs without a command.
pub(crate) fn print_help() -> io::Result<ExitCode> {
    Cli::command().print_help()?;
    Ok(ExitCode::SUCCESS)
}
```

Replace `crates/lmx/src/main.rs`:

```rust
//! # lmx
//!
//! Command of the LimaNix guest owner. People run it inside the VM; the LimaNix host runs it over
//! management SSH with `--json` and reads the [host contract](https://github.com/limanix/lmx/blob/main/docs/contract.md).
//!
//! | Command       | Kind  | Answers                                                     |
//! |---------------|-------|-------------------------------------------------------------|
//! | `lmx status`  | facts | generations, disk, interfaces and failed units              |
//! | `lmx version` | facts | the binary version and the host contract it speaks          |
//!
//! Facts are read in the caller's process with the caller's privileges and need no daemon.
//!
//! ## Test hooks
//!
//! Two environment variables let tests point the binary at prepared files. `sudo` drops both by
//! default, so the host never sets them by accident.
//!
//! | Variable          | Replaces                                                   |
//! |-------------------|------------------------------------------------------------|
//! | `LMX_CONFIG`      | [`lmx_model::CONFIG_PATH`]                                 |
//! | `LMX_SYSTEM_ROOT` | `/` for generation markers and the store path              |
#![forbid(unsafe_code)]

mod cli;
mod format;
mod output;
mod status;
mod system;
mod version;

use std::{io, process::ExitCode};

use clap::Parser;

use crate::{
    cli::{Cli, Command},
    system::System,
};

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Command::Status(args)) => status::run(&System::from_environment(), &args),
        Some(Command::Version(args)) => version::run(&args),
        None => cli::print_help(),
    };

    match result {
        Ok(code) => code,
        // Standard output was closed by its reader, as in `lmx status | true`: nobody is left to
        // read an answer or an error. clap ends `--help` the same way.
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lmx: {error}");
            ExitCode::from(output::FAILURE)
        }
    }
}
```

**Step 4: Run every test, lint and documentation build**

```bash
cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml --workspace --locked
cargo +1.90.0 clippy --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo +1.90.0 doc --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml --workspace --no-deps --document-private-items --locked
cargo +1.90.0 fmt --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml --all -- --check
```

Expected:
- 38 tests pass in total: model 10, facts 17, lmx unit 4, lmx integration 7.
- clippy and rustdoc report nothing.
- `fmt` prints no diff.

**Step 5: Try it on the host**

```bash
/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/target/debug/lmx status
```

Expected on macOS without Nix: `Generation desired unknown…` and `Disk unknown`. The Problem lines explain the
missing configuration, the disk problem (`cannot read guest disk usage…`) and the missing `ip` and `systemctl`. The
command exits 0.

**Step 6: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add crates/lmx
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Add lmx status with facts and problems"
```

---

## Task 11: Taskfile and containerized checks

**Files:**
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Taskfile.yml`
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/.taskrc.yml`

**Step 1: Write the Taskfile**

```yaml
# yaml-language-server: $schema=https://taskfile.dev/schema.json
version: '3.53.1'

includes:
  rust:
    taskfile: https://raw.githubusercontent.com/mr-chelyshkin/tasks/v0.0.5/taskfiles/rust/Taskfile.yml
    internal: true
  markdown:
    taskfile: https://raw.githubusercontent.com/mr-chelyshkin/tasks/v0.0.5/taskfiles/markdown/Taskfile.yml
    internal: true

vars:
  markdown_paths: README.md ARCHITECTURE.md docs/contract.md
  version:
    sh: sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)"/\1/p' Cargo.toml | head -n 1

tasks:
  ci:
    desc: List CI tasks.
    cmds:
      - task -l | grep "ci/"

  ci/rust-fmt:
    desc: Check Rust formatting.
    cmds:
      - task: rust:fmt
        vars: { FMT_ARGS: '--all --check' }

  ci/rust-clippy:
    desc: Lint every target with Clippy and deny warnings.
    cmds:
      - task: rust:clippy
        vars: { CLIPPY_ARGS: '--workspace --all-targets --locked -- -D warnings' }

  ci/rust-test:
    desc: Run unit and integration tests.
    cmds:
      - task: rust:test
        vars: { TEST_ARGS: '--workspace --locked' }

  ci/rust-docs:
    desc: Build API documentation, including private items, and deny rustdoc warnings.
    cmds:
      - task: rust:doc/build
        vars: { DOC_ARGS: '--workspace --no-deps --document-private-items --locked' }

  ci/rust-audit:
    desc: Scan dependencies for published advisories.
    cmds:
      - task: rust:audit

  ci/markdown-fmt:
    desc: Check Markdown formatting without modifying files.
    cmds:
      - task: markdown:fmt
        vars:
          FMT_PATHS: '{{.markdown_paths}}'
          TASK_CONTEXT:
            ref: 'merge (dict) (dict "CONTAINER_MOUNT_MODE" "ro") (.TASK_CONTEXT | default .)'

  rust/fix:
    desc: Format Rust sources.
    cmds:
      - task: rust:fmt/fix
        vars: { FMT_ARGS: '--all' }

  markdown/fix:
    desc: Format Markdown with the settings used in CI.
    cmds:
      - task: markdown:fmt/fix
        vars:
          FMT_PATHS: '{{.markdown_paths}}'

  release/build:
    desc: Build static release archives and checksums into dist/.
    cmds:
      - task: rust:_cargo/tool
        vars:
          RUSTFLAGS: -D warnings
          CMD: >-
            sh -eu -c '
              version="$1"
              rm -rf dist
              mkdir -p dist
              for system in aarch64-linux x86_64-linux; do
                case "$system" in
                  aarch64-linux) target=aarch64-unknown-linux-musl ;;
                  x86_64-linux) target=x86_64-unknown-linux-musl ;;
                esac
                cargo build --release --locked --target "$target" -p lmx
                name="lmx-$version-$system"
                mkdir -p "dist/$name"
                install -m 0755 "$CARGO_TARGET_DIR/$target/release/lmx" "dist/$name/lmx"
                install -m 0644 LICENSE "dist/$name/LICENSE"
                tar --sort=name --owner=0 --group=0 --numeric-owner --mtime=@0 -C dist -cf "dist/$name.tar" "$name"
                gzip -9n "dist/$name.tar"
                (cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
                rm -rf "dist/$name"
              done
            ' release {{.version}}
```

`release/build` is exercised in Task 13. It needs the musl targets that Task 12 adds to the CI image.

**Step 2: Trust the shared Taskfiles and cache them for a day** (`.taskrc.yml`)

This is the same as in `client`, `modules` and `docs`. Without it, `task` without `--yes` asks for confirmation
before every remote include.

```yaml
remote:
  cache-expiry: 24h
  trusted-hosts:
    - raw.githubusercontent.com
```

**Step 3: Run the checks in the container**

```bash
task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-fmt
task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-clippy
task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-test
task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-docs
task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-audit
```

Expected:
- Each task finishes without errors.
- `ci/rust-test` shows the same 38 passing tests, this time on Linux.
- `ci/rust-audit` reports no vulnerabilities.

**Step 4: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add Taskfile.yml .taskrc.yml
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Add Taskfile checks"
```

---

## Task 12: musl targets in the CI Rust image (repository `images`)

This task is in another repository: `/Users/igoss/Desktop/projects/images`. Publishing the image is the user's
pipeline: a push to `main` rebuilds `ghcr.io/mr-chelyshkin/ci/rust:1.90.0`. Prepare the change, then ask the user
to merge it.

**Files:**
- Modify: `/Users/igoss/Desktop/projects/images/ci/rust/Dockerfile`
- Modify: `/Users/igoss/Desktop/projects/images/README.md` (the `ci/rust` row)

**Step 1: Confirm the gap**

```bash
docker run --rm ghcr.io/mr-chelyshkin/ci/rust:1.90.0 rustup target list --installed
```

Expected: only the host target, without `*-unknown-linux-musl`.

**Step 2: Add the targets**

In `ci/rust/Dockerfile`, extend the toolchain step of the final stage:

```dockerfile
RUN test "$(rustc --version | cut -d ' ' -f 2)" = "${RUST_VERSION}" \
  && rustup component add --toolchain "${RUST_VERSION}" rustfmt clippy \
  && rustup target add --toolchain "${RUST_VERSION}" aarch64-unknown-linux-musl x86_64-unknown-linux-musl
```

In `README.md`, extend the `ci/rust` row and realign the table:

```markdown
| [`ci/rust`](ci/rust)           | `1.90.0`  | Rust `1.90.0`, rustfmt, Clippy, cargo-audit `0.22.0`, musl targets for aarch64 and x86_64 |
```

**Step 3: Build the image locally and check the targets**

```bash
docker build --build-arg BASE_IMAGE=rust:1.90.0-slim --build-arg CARGO_AUDIT_VERSION=0.22.0 \
  --build-arg RUST_VERSION=1.90.0 -t ci-rust-musl-check /Users/igoss/Desktop/projects/images/ci/rust
docker run --rm ci-rust-musl-check rustup target list --installed
```

Expected: the host target plus `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`.

**Step 4: Commit on a branch and hand over**

```bash
git -C /Users/igoss/Desktop/projects/images switch -c rust-musl-targets
git -C /Users/igoss/Desktop/projects/images add ci/rust/Dockerfile README.md
git -C /Users/igoss/Desktop/projects/images commit -m "Add musl targets to the Rust CI image"
```

Ask the user to merge and publish. Task 13 needs the published image.

---

## Task 13: Release archives

The archive layout is the delivery contract for M1c. The platform pins one SHA-256 per system and installs `lmx`
from `lmx-<version>-<system>/`.

**Files:** none new. `release/build` already exists in `Taskfile.yml`.

**Step 1: Build the archives**

After the image from Task 12 is published, run:

```bash
docker pull ghcr.io/mr-chelyshkin/ci/rust:1.90.0
task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes release/build
ls -l /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/dist
```

Expected files in `dist/`:
- `lmx-0.1.0-aarch64-linux.tar.gz` and `lmx-0.1.0-x86_64-linux.tar.gz`, each about 0.5 MB;
- a matching `.sha256` file for each archive.

Each archive contains `lmx-0.1.0-<system>/lmx` and `LICENSE`. Entries are owned by `0/0` with a zero timestamp.

**Step 2: Run the static binaries on Linux**

```bash
tar -xzf /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/dist/lmx-0.1.0-aarch64-linux.tar.gz -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/dist
docker run --rm --platform linux/arm64 -v /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/dist/lmx-0.1.0-aarch64-linux:/b:ro busybox /b/lmx status --json
rm -rf /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/dist/lmx-0.1.0-aarch64-linux
```

Expected: one JSON line with `"contract":1,"ok":true` and `problems` for the configuration, disk, `ip` and
`systemctl`. BusyBox has no store, no `ip -j` and no `systemd`. The point is that a static binary starts on a bare
Linux.

**Step 3: Nothing to commit**

`dist/` is ignored. If Step 1 needed a Taskfile fix, commit it with the message "Fix release build".

---

## Task 14: GitHub workflows

**Files:**
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/.github/workflows/pr.yml`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/.github/workflows/release.yml`

**Step 1: Pull-request checks**

The checks run every CI task, plus `release/build`, so a broken release surfaces before tagging:

```yaml
name: Pull request checks

on:
  pull_request:
    types: [opened, synchronize, reopened]

concurrency:
  group: pr-${{ github.ref }}
  cancel-in-progress: true

permissions:
  contents: read

jobs:
  check:
    name: ${{ matrix.command }}
    runs-on: ubuntu-24.04
    timeout-minutes: 30
    strategy:
      fail-fast: false
      matrix:
        command:
          - ci/rust-fmt
          - ci/rust-clippy
          - ci/rust-test
          - ci/rust-docs
          - ci/rust-audit
          - ci/markdown-fmt
          - release/build
    steps:
      - uses: actions/checkout@v6
        with:
          persist-credentials: false
      - name: Cache Cargo registry and build results
        if: matrix.command != 'ci/markdown-fmt'
        uses: actions/cache@v5
        with:
          path: .cache/rust
          key: rust-${{ runner.os }}-${{ runner.arch }}-${{ matrix.command }}-${{ hashFiles('Cargo.lock', 'Cargo.toml', 'rust-toolchain.toml') }}
          restore-keys: |
            rust-${{ runner.os }}-${{ runner.arch }}-${{ matrix.command }}-
      - name: Run ${{ matrix.command }}
        uses: mr-chelyshkin/actions/invoke-taskfile@v1
        with:
          command: ${{ matrix.command }}
```

**Step 2: Tag releases**

A `vX.Y.Z` tag must point to a commit on `main` and match the workspace version. The workflow then publishes the
archives and checksums:

```yaml
name: Release

on:
  push:
    tags: ['v*', '!v*\+*']

concurrency:
  group: release
  cancel-in-progress: false

permissions:
  contents: read

jobs:
  publish:
    name: Publish ${{ github.ref_name }}
    runs-on: ubuntu-24.04
    timeout-minutes: 30
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v6
        with:
          fetch-depth: 0
          persist-credentials: false
      - name: Check that the tag is on main
        uses: mr-chelyshkin/actions/check-tag-branch@v1
        with:
          branch: main
      - name: Check that the tag matches the workspace version
        env:
          TAG: ${{ github.ref_name }}
        run: |
          set -euo pipefail
          version=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
          if [ "v$version" != "$TAG" ]; then
            echo "::error::Tag $TAG does not match workspace version $version"
            exit 1
          fi
      - name: Build release archives
        uses: mr-chelyshkin/actions/invoke-taskfile@v1
        with:
          command: release/build
      - name: Create the GitHub release
        env:
          GH_TOKEN: ${{ github.token }}
          TAG: ${{ github.ref_name }}
        run: gh release create "$TAG" dist/*.tar.gz dist/*.sha256 --title "lmx $TAG" --generate-notes --verify-tag
```

**Step 3: Validate the YAML**

```bash
ruby -e 'require "yaml"; ARGV.each { |f| YAML.load_file(f); puts "ok #{f}" }' /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/.github/workflows/*.yml /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Taskfile.yml
```

Expected: `ok` for each file.

**Step 4: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add .github
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Add pull-request checks and tag releases"
```

---

## Task 15: Documentation

The README, contributor map and contract page follow taskvisor's structure:
- the problem first;
- tables for choices;
- boundaries worth knowing early;
- a contributor map with a source table.

**Files:**
- Create: `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/README.md`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/ARCHITECTURE.md`, `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx/docs/contract.md`

**Step 1: README**

````markdown
# lmx

[![License: Apache-2.0](https://img.shields.io/github/license/limanix/lmx?label=license)](LICENSE)

> **The owner of a LimaNix VM from inside: one command for people in the guest and one versioned contract for the host.**

`lmx` runs inside every [LimaNix](https://limanix.dev) guest.
It reports what the guest really is and answers the LimaNix host with versioned JSON over management SSH.

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

| Command       | Answers                                                                                   |
| ------------- | ----------------------------------------------------------------------------------------- |
| `lmx status`  | Desired, built and booted generations; store disk usage; interfaces; failed systemd units |
| `lmx version` | The binary version and the host contract version                                          |

Add `--json` to answer with the [host contract](docs/contract.md).
`lmx status` reads every fact independently: an unreadable fact is reported as a problem, and the others are still answered.

## Boundaries worth knowing early

- `lmx` has no network listener. The host reaches it only through management SSH.
- Facts are read in the caller's process with the caller's privileges and need no daemon.
- Configuration comes only from NixOS (`/etc/lmx/config.json`), never from the host at runtime.
- Release binaries are static musl executables for `aarch64` and `x86_64` Linux.

## Development

Requirements: [Task](https://taskfile.dev/docs/installation) 3.53.1+, Git and Docker.
Tasks run Cargo in the [`ci/rust`](https://github.com/mr-chelyshkin/images) image, so CI and local checks use one toolchain.

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

**Step 2: Contributor map** (`ARCHITECTURE.md`)

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
person or host ──► lmx (binary) ──► lmx-facts readers ──► statvfs, ip, systemctl, markers
                         │
                         └──► lmx-model::Envelope<T> ──► text or JSON on standard output
```

`lmx-model` holds every value that crosses a boundary: the configuration written by NixOS and the answers read by the host.
`lmx-facts` reads the running system. The `lmx` binary parses the command line, combines facts, and renders them.

## Boundaries to preserve

- Values that cross a process boundary belong in `lmx-model`; a field added elsewhere is not part of any contract. The `lmx version` answer still lives in the binary and moves to `lmx-model` in M1b.
- `lmx-facts` performs reads only. It never changes the system and never needs a daemon.
- A reader that runs a program splits process I/O from a pure parser; tests cover the parser with fixed output.
- The host contract changes only as described in [Change the host contract](docs/contract.md#change-the-host-contract).
- Configuration comes from NixOS. The binaries never accept configuration from the host at runtime.
- Every crate forbids unsafe Rust with `#![forbid(unsafe_code)]`.

## Source map

| Area              | Responsibility                                  | Start here                                            |
| ----------------- | ----------------------------------------------- | ----------------------------------------------------- |
| Contract types    | Configuration, envelope, error codes and status | [`lmx-model/src/lib.rs`](crates/lmx-model/src/lib.rs) |
| Fact readers      | Disk, generations, network and failed units     | [`lmx-facts/src/lib.rs`](crates/lmx-facts/src/lib.rs) |
| Command line      | Commands, output selection and exit codes       | [`lmx/src/main.rs`](crates/lmx/src/main.rs)           |
| Status            | Collecting facts and rendering them             | [`lmx/src/status.rs`](crates/lmx/src/status.rs)       |
| Contract examples | Published answers of each contract version      | [`contract/v1/`](contract/v1)                         |

Files outside `crates/` provide executable context:

| Path                                      | Purpose                                                 |
| ----------------------------------------- | ------------------------------------------------------- |
| [`crates/lmx/tests/`](crates/lmx/tests)   | The command-line contract against a prepared guest tree |
| [`Taskfile.yml`](Taskfile.yml)            | Checks and the release build                            |
| [`.github/workflows/`](.github/workflows) | Pull-request checks and tag releases                    |

## Add a fact

1. Add the value to `Status` in `lmx-model` with a doc comment, and update the examples in `contract/v1/`.
1. Add a reader module to `lmx-facts`: an I/O function and a pure parser with fixture tests.
1. Collect it in `lmx/src/status.rs` with `record`, so a failure becomes a problem instead of an error.
1. Render it in the text output and extend `crates/lmx/tests/cli.rs`.

A new optional field is a compatible change. Renaming or removing a field needs a new contract version.
````

**Step 3: Host contract** (`docs/contract.md`)

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
| `network.unreachable`  | A required destination, such as the binary cache, is unreachable   |
| `permission.denied`    | The caller is not allowed to run the operation                     |
| `generation.mismatch`  | The mounted inputs belong to a different generation than requested |
| `contract.unsupported` | The caller requested a contract version this binary does not speak |

`lmx status` and `lmx version` do not use these codes or the exit statuses `3` and `130`; later commands do.

## `lmx status`

`data` describes the guest. Every fact is optional: an unreadable fact is `null` and its reason is listed in `problems`.

| Field          | Meaning                                                                                                          |
| -------------- | ---------------------------------------------------------------------------------------------------------------- |
| `generations`  | `desired` (mounted at `/mnt/limanix`), `built` (system profile), `booted` (running system)                       |
| `disk`         | `bytes`, `free_bytes` (including the root reserve), `available_bytes`, `inodes`, `free_inodes`                   |
| `interfaces`   | `name`, lowercase `mac` (`null` without a hardware address), and global-scope `ipv4` addresses of each interface |
| `failed_units` | Names of failed systemd units                                                                                    |
| `problems`     | `fact` and `message` for each fact that could not be read in full; omitted when empty                            |

A problem's `fact` names the field that is `null` or incomplete, or is `config` when `/etc/lmx/config.json` cannot be read.
Without the configuration, `ip` and `systemctl` are looked up in `PATH`, so a `config` problem marks a degraded answer.

A generation is `null` when its stage has no valid marker, for example on a system built before `lmx` existed.
A marker that exists but cannot be read also gives `null` and adds a `generations` problem.
The host compares the three generations to tell whether a build or a restart is still needed.

Examples: [complete](../contract/v1/status.json), [partial](../contract/v1/status-partial.json).

## `lmx version`

`data` has `version`, the release version of the binary, and `contract`, the contract version it speaks.

## Change the host contract

- Adding an optional field, a new command or a new error code is compatible and keeps the version.
  Hosts treat an unknown code as a generic failure.
- Renaming, removing or changing the meaning of a field needs a new contract version.
- Every version keeps its examples in `contract/v<version>/`; tests decode and re-encode them without loss.
- The LimaNix client pins an `lmx` release and tests its decoders against that release's examples.
````

**Step 4: Check Markdown formatting**

```bash
task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/markdown-fmt
```

Expected: no changes are required. If it fails, run `task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes markdown/fix` and review the diff.

**Step 5: Commit**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx add README.md ARCHITECTURE.md docs/contract.md
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx commit -m "Document lmx, its contributor map and the host contract"
```

---

## Task 16: Final verification and hand-over

**Step 1: Run everything from a clean cache**

```bash
rm -rf /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/.cache/rust/target
task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-fmt && task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-clippy && task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-test && task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-docs && task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/rust-audit && task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes ci/markdown-fmt && task --dir /Users/igoss/Desktop/lima-personal-shared/limanix/lmx --yes release/build
```

Expected: every task succeeds and `dist/` contains two archives and two checksums.

**Step 2: Review the branch**

```bash
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx log --oneline main..feat/m1a-foundation
git -C /Users/igoss/Desktop/lima-personal-shared/limanix/lmx diff --stat main...feat/m1a-foundation
```

Expected: about 16 commits. The diff touches only `.gitignore`, `.cargo`, `.github`, `contract`, `crates`, `docs`,
the manifests, the Taskfile, `.taskrc.yml` and the three Markdown files.

**Step 3: Hand over**

- Ask the user whether to push `feat/m1a-foundation` and open a pull request.
- Remind them that the image change from Task 12 must be published before the `release/build` check can pass in CI.

---

## Out of scope for M1a

- **M1b:**
  - `help`, `info`, `welcome`;
  - `clipboard` (`pbcopy`/`pbpaste`) and `session` (`limanix-session`);
  - multi-call dispatch on `argv[0]`;
  - mounts, memory, CPUs and kernel facts;
  - the theme palette;
  - `Version` (the `lmx version` answer) in `lmx-model`, so every contract value lives there.
- **M1c:**
  - `generation` in `runtime.json`;
  - `/etc/lmx/config.json` and the pinned `fetchurl` package in the platform base;
  - removal of the shell scripts;
  - `limanix list` through `lmx status --json` with a fallback;
  - the per-system SHA-256 pins, taken from the release workflow's `.sha256` files; it builds on amd64 runners,
    and whether arm64 and amd64 hosts produce identical archives is unchecked.
- **JSON Schema of contract v1** (design section 8): deferred; the golden examples in `contract/v1/` define v1 until then.
- **Timeouts on the host mount:** generation markers under `/mnt/limanix` are read without a timeout, so a hung virtiofs
  mount would delay `lmx status`. Revisit when `lmxd` reads facts in M2.
- **Before Rust code decodes envelopes (M2, `lmx` ↔ `lmxd`):** give `ErrorCode` a catch-all, or decode `code` as a
  string. Today an unknown code fails the whole envelope.
- **The first tag.** Tag `v0.1.0` only after M1b, because the guest's public commands must keep working when M1c
  switches the platform to the binary.

## Follow-ups from the final review

These are not defects in M1a. Include them when you write the next plans.

- **M1b:**
  - **Help output.** `lmx`, `lmx help`, `lmx --help` and `lmx -h` must show the guest help page that `lmx.sh` shows
    today. Right now the binary prints clap help, and `lmx help` exits 2.
  - **`welcome` and `info` need their own time budget and presentation.** `welcome` runs at every interactive shell
    start. For the dev user, `generations` is always an expected `Permission denied` problem.
  - **Doc and test fixes:**
    - In `lmx-facts/src/lib.rs`, the runner docs should mention the `PATH` fallback.
    - In `generations.rs`, `under()` is for tests only: absolute store symlinks resolve on the inspecting machine.
    - Add a test that the `disk` problem uses the fact name `disk`.
    - Render inodes only when `disk.inodes > 0`.
- **M1c:**
  - **Configuration file.** Render `/etc/lmx/config.json` world-readable, with `tools` as store paths.
  - **Decoding.** Decode `data.disk` straight into `domain.DiskUsage`, and reuse the MAC-and-IPv4 selection from
    `address.go`.
  - **When to fall back to the old probes.** Fall back when stdout holds no JSON answer, not when the exit status is
    nonzero. `sudo: lmx: command not found` also exits 1.
- **M2:**
  - **Tool runners.** `lmxd` must not use the CLI tool runners, because a timed-out tool and its thread stay alive. Feed
    `network::parse` and `units::parse` from solti-exec output instead.
  - **Commands with no result** answer `{}`, never `"data": null`.
  - **Configuration compatibility.** Before M3 runs a pinned `lmx` against an older `/etc/lmx/config.json`, add new
    schema-1 fields with `#[serde(default)]` or bump `CONFIG_SCHEMA`.
  - **Release archive.** `release/build` adds `lmxd` to the same `lmx-<version>-<system>/` directory.
- **CI (optional):** add `push: branches: [main]` to `pr.yml`, so `main` saves caches that pull requests can reuse.
