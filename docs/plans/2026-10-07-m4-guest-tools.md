# M4: doctor, net check, logs, short status and theme in `lmx` — implementation plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Give people and the host diagnostics inside the guest: `lmx doctor`, `lmx net check PORT`, `lmx logs KIND` and `lmx status --short`, with colors from the declared theme and journal fields of `lmxd` that `lmx logs` can find.

**Architecture:** Every new command is a fact: it runs in the caller's process with the caller's privileges and works while `lmxd` is down. `doctor` and `--short` ask `lmxd` for its conditions with a timeout; `net check` reads `/proc` and the declared ports; `logs` reads journald through `journalctl`. `lmxd` only names its journal fields, without tracing-journald's `F_` prefix, through a new option of `solti-observe`.

**Tech Stack:** Rust 1.90.0 (edition 2024), Solti (local SDK by path), clap, serde_json.

Design: [M4 design](2026-10-07-m4-guest-tools-design.md). Background: [guest owner
design](2026-10-06-guest-owner-design.md), [M3 plan](2026-10-07-m3-apply-finalize.md).

## Before you start

- **Repository:** `/Users/igoss/Desktop/lima-personal-shared/limanix/lmx`, branch `feat/m1a-foundation`, with M3 committed (`69407b3`).
- **Never commit,** in this repository or in the SDK. The user commits; the suggested commits are at the end.
- **Solti comes from the local SDK by path.** Task 1 changes `solti-observe` in `/Users/igoss/Desktop/projects/solti/sdk`; leave the user's own changes there, such as the version bump in `Cargo.toml`, alone.
- **Commands** run on the host with the pinned toolchain. Use your own `--target-dir` if an IDE builds the repository at the same time.
- **Every task ends green:** formatting, Clippy with `-D warnings` on every target, all tests, and rustdoc with `-D warnings`. Code arrives with its first caller, so no task leaves dead code.

| Task | Delivers |
|---|---|
| 1 | SDK: a journald field prefix option in `solti-observe` |
| 2 | Contract: `network.ports`, `theme` and `tools.journalctl` in the configuration; check records |
| 3 | Facts: listening sockets and their processes; journal records of `lmxd` tasks |
| 4 | `lmxd`: journal fields `LMX_TASK`, `LMX_KIND` and `LMX_GENERATION` |
| 5 | `lmx`: colors from the declared theme |
| 6 | `lmx doctor` and `lmx status --short` |
| 7 | `lmx net check` and `lmx logs` |
| 8 | Documentation |

---

## Task 1: A journald field prefix option in `solti-observe`

**Files:**
- Modify: `/Users/igoss/Desktop/projects/solti/sdk/crates/solti-observe/src/logger/config.rs`, `/Users/igoss/Desktop/projects/solti/sdk/crates/solti-observe/src/logger/log.rs`, `/Users/igoss/Desktop/projects/solti/sdk/crates/solti-observe/README.md`, `/Users/igoss/Desktop/projects/solti/sdk/crates/solti-observe/examples/text_logging.rs`, `/Users/igoss/Desktop/projects/solti/sdk/crates/solti/examples/operations_observe.rs`, `/Users/igoss/Desktop/projects/solti/sdk/docs/observability.md`

tracing-journald names an event field `request_id` as `F_REQUEST_ID`: its default prefix is `F`, and `solti-observe` has
no way to change it. `lmxd` wants `LMX_TASK`, which people and `lmx logs` filter by. The new field keeps `Some("F")` by
default, so nothing changes for other users of the SDK. An empty prefix means none, because tracing-journald would start
every field with `_`, which journald drops. The user committed this change to the SDK as `c27640f` and `ad3362e`; the
plan shows it for reference, and its replay does not repeat it.

**Step 1: Add the field with its default and documentation**

The whole change, as a diff of the SDK checkout:

```diff
diff --git a/crates/solti-observe/README.md b/crates/solti-observe/README.md
index 39ee684..c9b9af8 100644
--- a/crates/solti-observe/README.md
+++ b/crates/solti-observe/README.md
@@ -50,13 +50,14 @@ For local text or JSON timestamps, `init_logger` detects the offset before globa
 
 ## Configuration
 
-| Field          | Default | Used by                                  |
-|----------------|---------|------------------------------------------|
-| `format`       | `Text`  | Backend selection                        |
-| `level`        | `info`  | Every backend                            |
-| `timezone`     | `Utc`   | Text and JSON timestamps                 |
-| `with_targets` | `true`  | Text and JSON event targets              |
-| `use_color`    | `true`  | Text output on an interactive terminal   |
+| Field                   | Default     | Used by                                               |
+|-------------------------|-------------|-------------------------------------------------------|
+| `format`                | `Text`      | Backend selection                                     |
+| `level`                 | `info`      | Every backend                                         |
+| `timezone`              | `Utc`       | Text and JSON timestamps                              |
+| `with_targets`          | `true`      | Text and JSON event targets                           |
+| `use_color`             | `true`      | Text output on an interactive terminal                |
+| `journald_field_prefix` | `Some("F")` | Journald field names; `None` or `""` drops the prefix |
 
 Missing Serde fields use these defaults. Unknown fields are rejected, including
 misspelled setting names.
diff --git a/crates/solti-observe/examples/text_logging.rs b/crates/solti-observe/examples/text_logging.rs
index 9cbb1b3..3984857 100644
--- a/crates/solti-observe/examples/text_logging.rs
+++ b/crates/solti-observe/examples/text_logging.rs
@@ -53,6 +53,7 @@ fn main() -> ExampleResult {
         timezone: LoggerTimeZone::Utc,
         with_targets: true,
         use_color: false,
+        ..LoggerConfig::default()
     };
     println!(
         "[config] format={}, level={}, timezone={}, targets=true, color=false.",
diff --git a/crates/solti-observe/src/logger/config.rs b/crates/solti-observe/src/logger/config.rs
index 326683e..1700db7 100644
--- a/crates/solti-observe/src/logger/config.rs
+++ b/crates/solti-observe/src/logger/config.rs
@@ -16,13 +16,14 @@ use crate::logger::object::{LoggerFormat, LoggerLevel, LoggerTimeZone};
 ///
 /// ## Defaults
 ///
-/// | Field          | Default | Used by                                |
-/// |----------------|---------|----------------------------------------|
-/// | `format`       | `Text`  | Backend selection                      |
-/// | `level`        | `info`  | Every backend                          |
-/// | `timezone`     | `Utc`   | Text and JSON timestamps               |
-/// | `with_targets` | `true`  | Text and JSON event targets            |
-/// | `use_color`    | `true`  | Text output on an interactive terminal |
+/// | Field                   | Default     | Used by                                |
+/// |-------------------------|-------------|----------------------------------------|
+/// | `format`                | `Text`      | Backend selection                      |
+/// | `level`                 | `info`      | Every backend                          |
+/// | `timezone`              | `Utc`       | Text and JSON timestamps               |
+/// | `with_targets`          | `true`      | Text and JSON event targets            |
+/// | `use_color`             | `true`      | Text output on an interactive terminal |
+/// | `journald_field_prefix` | `Some("F")` | Journald field names                   |
 ///
 /// Missing Serde fields use these defaults. Unknown fields are rejected so a
 /// misspelled setting cannot silently retain its default value.
@@ -55,6 +56,13 @@ pub struct LoggerConfig {
     ///
     /// Colors are used only for text written to an interactive terminal.
     pub use_color: bool,
+    /// Prefix of the journald names of event and span fields; `None` or an empty prefix writes the
+    /// field names alone.
+    ///
+    /// Journald fields are uppercase, so the field `request_id` becomes `F_REQUEST_ID` with the
+    /// default prefix and `REQUEST_ID` without one. A service whose operators filter the journal by
+    /// its own fields, such as `journalctl REQUEST_ID=…`, can drop the prefix.
+    pub journald_field_prefix: Option<String>,
 }
 
 impl Default for LoggerConfig {
@@ -65,6 +73,7 @@ impl Default for LoggerConfig {
             timezone: LoggerTimeZone::default(),
             with_targets: true,
             use_color: true,
+            journald_field_prefix: Some("F".into()),
         }
     }
 }
@@ -92,6 +101,7 @@ mod tests {
         assert_eq!(config.level.as_str(), "info");
         assert!(config.with_targets);
         assert!(config.use_color);
+        assert_eq!(config.journald_field_prefix.as_deref(), Some("F"));
     }
 
     #[test]
@@ -102,6 +112,7 @@ mod tests {
             level: "debug".parse().unwrap(),
             with_targets: false,
             use_color: false,
+            journald_field_prefix: None,
         };
 
         let json = serde_json::to_string(&config).unwrap();
@@ -112,6 +123,7 @@ mod tests {
         assert_eq!(config.use_color, parsed.use_color);
         assert_eq!(config.format, parsed.format);
         assert_eq!(config.timezone, parsed.timezone);
+        assert_eq!(parsed.journald_field_prefix, None);
     }
 
     #[test]
diff --git a/crates/solti-observe/src/logger/log.rs b/crates/solti-observe/src/logger/log.rs
index 6ed06fb..e7a957d 100644
--- a/crates/solti-observe/src/logger/log.rs
+++ b/crates/solti-observe/src/logger/log.rs
@@ -50,12 +50,19 @@ pub(super) fn logger_json(cfg: &LoggerConfig) -> Result<(), LoggerError> {
 
 /// Initializes the journald logger on Linux.
 ///
-/// Journald uses the configured level filter.
+/// Journald uses the configured level filter and field prefix.
 /// Its native layer owns record formatting.
 #[cfg(all(feature = "journald", target_os = "linux"))]
 pub(super) fn logger_journald(cfg: &LoggerConfig) -> Result<(), LoggerError> {
     let filter = cfg.level.to_env_filter();
-    let journald = tracing_journald::layer().map_err(LoggerError::JournaldInitFailed)?;
+    // An empty prefix would start every field with `_`, which journald drops; it means no prefix.
+    let prefix = cfg
+        .journald_field_prefix
+        .clone()
+        .filter(|prefix| !prefix.is_empty());
+    let journald = tracing_journald::layer()
+        .map_err(LoggerError::JournaldInitFailed)?
+        .with_field_prefix(prefix);
 
     let subscriber = tracing_subscriber::registry().with(filter).with(journald);
     init_subscriber(subscriber)
diff --git a/crates/solti/examples/operations_observe.rs b/crates/solti/examples/operations_observe.rs
index d8823c5..aebe8a4 100644
--- a/crates/solti/examples/operations_observe.rs
+++ b/crates/solti/examples/operations_observe.rs
@@ -69,6 +69,7 @@ solti: logging and supervised maintenance
         timezone: LoggerTimeZone::Local,
         with_targets: true,
         use_color: false,
+        ..LoggerConfig::default()
     })?;
     tracing::info!(
         target: "example::operations",
diff --git a/docs/observability.md b/docs/observability.md
index c5cc5d3..1bc603f 100644
--- a/docs/observability.md
+++ b/docs/observability.md
@@ -65,6 +65,7 @@ The snippet also needs the application's `tracing` dependency.
 | `timezone` | UTC | UTC or cached local offset for text and JSON timestamps. |
 | `with_targets` | `true` | Include event targets in text and JSON. |
 | `use_color` | `true` | ANSI only for text when stdout is an interactive terminal. |
+| `journald_field_prefix` | `"F"` | Prefix of journald field names, such as `F_REQUEST_ID`; `null` or `""` writes `REQUEST_ID`. |
 
 Serde fills missing fields from these defaults and rejects unknown fields.
 JSON never uses ANSI colors. Text and JSON timestamps are RFC 3339.
```

**Step 2: Run the SDK tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/projects/solti/sdk/Cargo.toml -p solti-observe --all-features`

Expected: 39 tests pass. The journald backend compiles only on Linux; the Linux check at the end builds it.

---

## Task 2: Configuration fields and check records

**Files:**
- Create: `crates/lmx-model/src/check.rs`, `contract/v1/doctor.json`, `contract/v1/net-check.json`
- Modify: `crates/lmx-model/src/config.rs`, `crates/lmx-model/src/lib.rs`, `crates/lmxd/src/daemon.rs`, `crates/lmxd/src/tasks.rs`, `crates/lmx/src/help.rs`, `crates/lmx/tests/cli.rs`, `crates/lmx/tests/owner.rs`

M1c renders three new configuration fields: the declared ports for `net check`, the theme for colors, and `journalctl`
for `logs`. `doctor` and `net check` answer with check records.

**Step 1: Write the failing test of the configuration**

In `crates/lmx-model/src/config.rs`, the configuration sample gains the fields. A color such as `"#89b4fa"` would end a
`r#"…"#` string, so the sample's delimiters become `r##"…"##`:

```rust
const SAMPLE: &str = r#"{
```

with:

```rust
const SAMPLE: &str = r##"{
```

and

```rust
        "health": {"units": ["sshd.service", "lmx.socket"]},
```

with:

```rust
        "health": {"units": ["sshd.service", "lmx.socket"]},
        "network": {"ports": {"tcp": [8080], "udp": []}},
        "theme": {"flavor": "mocha", "palette": {"blue": "#89b4fa", "red": "#f38ba8"}},
```

and

```rust
            "systemd_run": "/run/current-system/sw/bin/systemd-run"
        }
    }"#;
```

with:

```rust
            "systemd_run": "/run/current-system/sw/bin/systemd-run",
            "journalctl": "/run/current-system/sw/bin/journalctl"
        }
    }"##;
```

and the test reads them:

```rust
        assert_eq!(config.disk.minimum_percent, 10);
        assert_eq!(config.session.command, None);
```

with:

```rust
        assert_eq!(config.disk.minimum_percent, 10);
        assert_eq!(config.network.ports.tcp, [8080]);
        assert_eq!(config.theme.palette["blue"], "#89b4fa");
        assert_eq!(config.session.command, None);
```

**Step 2: Run it to see it fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model`

Expected: the tests do not compile: ``no field `network` on type `config::Config` ``.

**Step 3: Add the fields**

In `crates/lmx-model/src/config.rs`, the import:

```rust
use std::{
    fs, io,
    path::{Path, PathBuf},
};
```

with:

```rust
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};
```

the fields of `Config`:

```rust
    /// Health check of an applied generation.
    pub health: Health,
    /// Named-session provider selected by catalog modules.
```

with:

```rust
    /// Health check of an applied generation.
    pub health: Health,
    /// Network declaration of the guest.
    pub network: Network,
    /// Colors of the guest, chosen in the declaration.
    pub theme: Theme,
    /// Named-session provider selected by catalog modules.
```

the tool:

```rust
    pub systemd_run: String,
}
```

with:

```rust
    pub systemd_run: String,
    /// `journalctl`, used to read the history of `lmxd` tasks.
    pub journalctl: String,
}
```

and the types, before `ConfigError`:

```rust
/// Network declaration of the guest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Network {
    /// Ports the guest firewall opens, as NixOS evaluated `networking.firewall`: those of
    /// `network.ports` in `limanix.toml` and those that modules open.
    pub ports: Ports,
}

/// Ports the guest firewall opens.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ports {
    /// Open TCP ports.
    pub tcp: Vec<u16>,
    /// Open UDP ports.
    pub udp: Vec<u16>,
}

/// Colors of the guest, chosen in the declaration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    /// Catppuccin flavor, such as `mocha`.
    pub flavor: String,
    /// The flavor's colors by name, such as `blue`, as `#rrggbb`.
    pub palette: BTreeMap<String, String>,
}
```

Export them from `crates/lmx-model/src/lib.rs`:

```rust
pub use config::{
    CONFIG_PATH, CONFIG_SCHEMA, Config, ConfigError, DiskPolicy, Health, Session, Tools, User, Vm,
};
```

with:

```rust
pub use config::{
    CONFIG_PATH, CONFIG_SCHEMA, Config, ConfigError, DiskPolicy, Health, Network, Ports, Session,
    Theme, Tools, User, Vm,
};
```

**Step 4: Run the model tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-model`

Expected: 17 tests pass.

**Step 5: Add the check records**

`doctor` and `net check` answer with records of the same shape. `unknown` marks a part the caller may not read, and its
hint says to use `sudo`. Create the examples:

`contract/v1/doctor.json`:

```json
{
  "contract": 1,
  "ok": true,
  "data": {
    "checks": [
      {
        "check": "config",
        "status": "ok",
        "message": "/etc/lmx/config.json is valid; generation 0123456789ab."
      },
      {
        "check": "owner",
        "status": "ok",
        "message": "lmxd 0.1.0 answers."
      },
      {
        "check": "generations",
        "status": "warning",
        "message": "Generation 0123456789ab is built; restart the VM to boot it.",
        "hint": "Restart the VM from the Mac; limanix update does it."
      }
    ]
  }
}
```

`contract/v1/net-check.json`:

```json
{
  "contract": 1,
  "ok": true,
  "data": {
    "port": 8080,
    "protocol": "tcp",
    "checks": [
      {
        "check": "firewall",
        "status": "ok",
        "message": "TCP 8080 is open in the guest firewall."
      },
      {
        "check": "listener",
        "status": "failed",
        "message": "TCP 8080 listens on 127.0.0.1 only, so it is reachable only inside the guest.",
        "hint": "Make the application listen on 0.0.0.0 or the guest address."
      },
      {
        "check": "process",
        "status": "ok",
        "message": "python3 (pid 4242) holds the socket of user dev."
      }
    ]
  }
}
```

and `crates/lmx-model/src/check.rs`, whose test decodes and re-encodes them:

```rust
//! Answers of `lmx doctor` and `lmx net check`: one record per check.

use serde::{Deserialize, Serialize};

/// Outcome of one check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    /// The check passed.
    Ok,
    /// Something needs attention, but nothing is broken.
    Warning,
    /// Something is broken; the hint says what to do.
    Failed,
    /// The caller lacks the privileges to check; the hint says to use `sudo`.
    Unknown,
}

/// One check and its finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// Stable name of the check, such as `owner`.
    pub check: String,
    /// Outcome.
    pub status: CheckStatus,
    /// Finding for people.
    pub message: String,
    /// What to do next, when there is something to do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Check {
    /// A check named `check` with `status` and `message`, without a hint.
    #[must_use]
    pub fn new(check: &str, status: CheckStatus, message: impl Into<String>) -> Self {
        Self {
            check: check.to_owned(),
            status,
            message: message.into(),
            hint: None,
        }
    }

    /// The check with `hint`.
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// Answer of `lmx doctor`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Doctor {
    /// The checks in the order they ran.
    pub checks: Vec<Check>,
}

/// Transport protocol of a port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

impl Protocol {
    /// Name for people, such as `TCP`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        }
    }
}

/// Answer of `lmx net check`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetCheck {
    /// The port checked.
    pub port: u16,
    /// Its protocol.
    pub protocol: Protocol,
    /// The checks in the order they ran.
    pub checks: Vec<Check>,
}

#[cfg(test)]
mod tests {
    use crate::{CONTRACT_VERSION, Doctor, Envelope, NetCheck};

    /// The published examples decode and encode without loss.
    #[test]
    fn contract_examples_round_trip() {
        let example = include_str!("../../../contract/v1/doctor.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<Doctor> = serde_json::from_str(example).expect("example decodes");
        assert_eq!(envelope.contract, CONTRACT_VERSION);
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);

        let example = include_str!("../../../contract/v1/net-check.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<NetCheck> = serde_json::from_str(example).expect("example decodes");
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);
    }
}
```

In `crates/lmx-model/src/lib.rs`, the table of the crate documentation:

```rust
//! | [`Apply`]    | `lmx apply`, with [`ApplyEvent`]s       | the host and people     |
```

with:

```rust
//! | [`Apply`]    | `lmx apply`, with [`ApplyEvent`]s       | the host and people     |
//! | [`Doctor`]   | `lmx doctor`                            | the host and people     |
//! | [`NetCheck`] | `lmx net check`                         | the host and people     |
```

the module:

```rust
mod apply;
mod config;
```

with:

```rust
mod apply;
mod check;
mod config;
```

and the export:

```rust
pub use config::{
```

with:

```rust
pub use check::{Check, CheckStatus, Doctor, NetCheck, Protocol};
pub use config::{
```

**Step 6: Keep the other configurations complete**

Every configuration in the tests needs the fields. In the test configuration of `crates/lmxd/src/daemon.rs`:

```rust
            "health": {"units": ["sshd.service"]},
```

with:

```rust
            "health": {"units": ["sshd.service"]},
            "network": {"ports": {"tcp": [8080], "udp": []}},
            "theme": {"flavor": "mocha", "palette": {}},
```

and

```rust
                "systemd_run": tool("systemd-run")
            }
```

with:

```rust
                "systemd_run": tool("systemd-run"),
                "journalctl": tool("journalctl")
            }
```

and the configuration check rejects a relative `journalctl` too:

```rust
        ("systemd_run", &tools.systemd_run),
    ] {
```

with:

```rust
        ("systemd_run", &tools.systemd_run),
        ("journalctl", &tools.journalctl),
    ] {
```

In the test tools of `crates/lmxd/src/tasks.rs`:

```rust
                systemd_run: "/bin/systemd-run".into(),
```

with:

```rust
                systemd_run: "/bin/systemd-run".into(),
                journalctl: "/bin/journalctl".into(),
```

In the test configuration of `crates/lmx/src/help.rs`:

```rust
    use lmx_model::{Config, DiskPolicy, Health, Session, Tools, User, Vm};
```

with:

```rust
    use lmx_model::{Config, DiskPolicy, Health, Network, Session, Theme, Tools, User, Vm};
```

and

```rust
            health: Health { units: vec![] },
```

with:

```rust
            health: Health { units: vec![] },
            network: Network {
                ports: Default::default(),
            },
            theme: Theme {
                flavor: "mocha".into(),
                palette: Default::default(),
            },
```

and

```rust
                systemd_run: "systemd-run".into(),
```

with:

```rust
                systemd_run: "systemd-run".into(),
                journalctl: "journalctl".into(),
```

In the guest configuration of `crates/lmx/tests/cli.rs`:

```rust
            "health": {"units": ["sshd.service"]},
```

with:

```rust
            "health": {"units": ["sshd.service"]},
            "network": {"ports": {"tcp": [8080], "udp": []}},
            "theme": {"flavor": "mocha", "palette": {}},
```

and

```rust
                "systemd_run": "/run/current-system/sw/bin/systemd-run"
            }
```

with:

```rust
                "systemd_run": "/run/current-system/sw/bin/systemd-run",
                "journalctl": "/run/current-system/sw/bin/journalctl"
            }
```

and in `crates/lmx/tests/owner.rs`:

```rust
            "health": {"units": ["sshd.service"]},
```

with:

```rust
            "health": {"units": ["sshd.service"]},
            "network": {"ports": {"tcp": [8080], "udp": []}},
            "theme": {"flavor": "mocha", "palette": {}},
```

and

```rust
                "systemd_run": path("bin/systemd-run")
            }
```

with:

```rust
                "systemd_run": path("bin/systemd-run"),
                "journalctl": path("bin/journalctl")
            }
```

**Step 7: Run the workspace tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml --workspace`

Expected: 154 tests pass, including `contract_examples_round_trip` of the check records.

---

## Task 3: Listening sockets and journal records

**Files:**
- Create: `crates/lmx-facts/src/sockets.rs`, `crates/lmx-facts/src/journal.rs`
- Modify: `crates/lmx-facts/src/command.rs`, `crates/lmx-facts/src/lib.rs`

Two new readers. `sockets` reads the kernel's socket tables, which every user may read, and finds the processes that
hold a socket; only root sees the descriptors of other users' processes. `journal` runs `journalctl -o json` and keeps
the records of `lmxd` tasks.

**Step 1: Read the socket tables**

Create `crates/lmx-facts/src/sockets.rs`. The kernel prints an address, which is in network order, as native-endian
32-bit words, so the bytes of each parsed word are its native bytes. The tests cover IPv4 and IPv6 listeners, UDP
sockets without a peer, and the search of `/proc/<pid>/fd` on a prepared tree:

```rust
//! Sockets that wait for connections or datagrams, and the processes that hold them.
//!
//! The kernel lists the sockets of each protocol in `/proc/net/{tcp,tcp6,udp,udp6}`, which every
//! user can read. A socket belongs to the processes with a descriptor of its inode in
//! `/proc/<pid>/fd`; only root can read the descriptors of other users' processes.

use std::{
    fs, io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::Path,
};

use lmx_model::Protocol;

use crate::FactError;

/// Root of the process file system inside a booted guest.
pub const PROC_PATH: &str = "/proc";

/// TCP state of a socket that accepts connections.
const TCP_LISTEN: u8 = 0x0A;

/// One socket that waits for connections or datagrams.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listener {
    /// Local address; `0.0.0.0` or `::` for every address.
    pub address: IpAddr,
    /// Local port.
    pub port: u16,
    /// User that created the socket.
    pub uid: u32,
    /// Inode that identifies the socket in `/proc/<pid>/fd`.
    pub inode: u64,
}

/// A process that holds a socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    /// Process ID.
    pub pid: u32,
    /// Command name, from `/proc/<pid>/comm`.
    pub command: String,
    /// Inode of the socket it holds.
    pub inode: u64,
}

/// Listeners of `protocol` on `port`, from the socket tables below `proc`.
///
/// A table that does not exist, such as `tcp6` without IPv6, has no listeners.
pub fn listeners(proc: &Path, protocol: Protocol, port: u16) -> Result<Vec<Listener>, FactError> {
    let tables = match protocol {
        Protocol::Tcp => ["tcp", "tcp6"],
        Protocol::Udp => ["udp", "udp6"],
    };
    let mut found = Vec::new();
    for name in tables {
        let table = match fs::read_to_string(proc.join("net").join(name)) {
            Ok(table) => table,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(FactError::Io {
                    what: "the socket table",
                    source,
                });
            }
        };
        found.extend(
            parse(&table, protocol)?
                .into_iter()
                .filter(|listener| listener.port == port),
        );
    }
    Ok(found)
}

/// Parses a socket table, keeping TCP sockets in `LISTEN` and UDP sockets without a peer.
pub fn parse(table: &str, protocol: Protocol) -> Result<Vec<Listener>, FactError> {
    let mut listeners = Vec::new();
    for line in table.lines().skip(1).filter(|line| !line.trim().is_empty()) {
        let malformed = || FactError::Parse {
            what: "socket table",
            detail: line.to_owned(),
        };
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (Some(local), Some(remote), Some(state), Some(uid), Some(inode)) = (
            fields.get(1),
            fields.get(2),
            fields.get(3),
            fields.get(7),
            fields.get(9),
        ) else {
            return Err(malformed());
        };
        let state = u8::from_str_radix(state, 16).map_err(|_| malformed())?;
        let (_, remote_port) = endpoint(remote).ok_or_else(malformed)?;
        let waiting = match protocol {
            Protocol::Tcp => state == TCP_LISTEN,
            Protocol::Udp => remote_port == 0,
        };
        if !waiting {
            continue;
        }
        let (address, port) = endpoint(local).ok_or_else(malformed)?;
        listeners.push(Listener {
            address,
            port,
            uid: uid.parse().map_err(|_| malformed())?,
            inode: inode.parse().map_err(|_| malformed())?,
        });
    }
    Ok(listeners)
}

/// Decodes `ADDRESS:PORT` of a socket table.
///
/// The kernel prints the address, which is in network order, as native-endian 32-bit words, and the
/// port as a number.
fn endpoint(field: &str) -> Option<(IpAddr, u16)> {
    let (address, port) = field.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let mut bytes = Vec::with_capacity(16);
    for word in address.as_bytes().chunks(8) {
        let word = u32::from_str_radix(std::str::from_utf8(word).ok()?, 16).ok()?;
        bytes.extend_from_slice(&word.to_ne_bytes());
    }
    let address = match bytes.len() {
        4 => IpAddr::V4(Ipv4Addr::from(<[u8; 4]>::try_from(bytes).ok()?)),
        16 => IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(bytes).ok()?)),
        _ => return None,
    };
    Some((address, port))
}

/// The processes below `proc` that hold any of the sockets `inodes`, in one search, and whether
/// every process could be searched.
pub fn holders(proc: &Path, inodes: &[u64]) -> Result<(Vec<Holder>, bool), FactError> {
    let targets: Vec<(String, u64)> = inodes
        .iter()
        .map(|inode| (format!("socket:[{inode}]"), *inode))
        .collect();
    let entries = fs::read_dir(proc).map_err(|source| FactError::Io {
        what: "the process list",
        source,
    })?;
    let mut holders = Vec::new();
    let mut complete = true;
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let descriptors = match fs::read_dir(entry.path().join("fd")) {
            Ok(descriptors) => descriptors,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                complete = false;
                continue;
            }
            // The process exited while the list was read.
            Err(_) => continue,
        };
        let mut held: Vec<u64> = descriptors
            .flatten()
            .filter_map(|descriptor| fs::read_link(descriptor.path()).ok())
            .filter_map(|link| {
                targets
                    .iter()
                    .find(|(target, _)| link.as_os_str() == target.as_str())
                    .map(|(_, inode)| *inode)
            })
            .collect();
        held.sort_unstable();
        held.dedup();
        if held.is_empty() {
            continue;
        }
        let command = fs::read_to_string(entry.path().join("comm"))
            .map(|command| command.trim_end().to_owned())
            .unwrap_or_default();
        holders.extend(held.into_iter().map(|inode| Holder {
            pid,
            command: command.clone(),
            inode,
        }));
    }
    holders.sort_by_key(|holder| (holder.pid, holder.inode));
    Ok((holders, complete))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    /// A TCP table with a loopback listener, a listener on every address, and a connection.
    const TCP: &str = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4242 1 0 100 0 0 10 0
   1: 00000000:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 11870 1 0 100 0 0 10 0
   2: 0A00020F:0016 0100020A:C350 01 00000000:00000000 02:00000AD7 00000000     0        0 12345 4 0 20 4 30 10 -1
";

    /// A TCP6 table with listeners on `::1` and on every address.
    const TCP6: &str = "\
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4243 1 0 100 0 0 10 0
   1: 00000000000000000000000000000000:0016 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 11872 1 0 100 0 0 10 0
";

    #[test]
    fn keeps_listening_sockets_with_their_addresses() {
        let listeners = parse(TCP, Protocol::Tcp).expect("valid table");
        assert_eq!(
            listeners,
            [
                Listener {
                    address: IpAddr::from([127, 0, 0, 1]),
                    port: 8080,
                    uid: 1000,
                    inode: 4242,
                },
                Listener {
                    address: IpAddr::from([0, 0, 0, 0]),
                    port: 22,
                    uid: 0,
                    inode: 11870,
                },
            ]
        );
        let listeners = parse(TCP6, Protocol::Tcp).expect("valid table");
        assert_eq!(listeners[0].address, IpAddr::V6(Ipv6Addr::LOCALHOST));
        assert_eq!(listeners[1].address, IpAddr::V6(Ipv6Addr::UNSPECIFIED));
    }

    #[test]
    fn a_udp_socket_without_a_peer_waits_for_datagrams() {
        let table = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode ref pointer drops
  1: 00000000:0044 00000000:0000 07 00000000:00000000 00:00000000 00000000     0        0 9861 2 0 0
  2: 0100007F:0035 0100007F:D431 01 00000000:00000000 00:00000000 00000000     0        0 9862 2 0 0
";
        let listeners = parse(table, Protocol::Udp).expect("valid table");
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].port, 68);
    }

    #[test]
    fn finds_the_processes_that_hold_a_socket() {
        let proc = tempfile::tempdir().expect("temporary /proc");
        for (pid, command, socket) in [("4242", "python3", 4242), ("7", "sshd", 11870)] {
            let fd = proc.path().join(pid).join("fd");
            fs::create_dir_all(&fd).expect("create fd");
            symlink(format!("socket:[{socket}]"), fd.join("3")).expect("link the socket");
            symlink("/dev/null", fd.join("0")).expect("link stdin");
            fs::write(proc.path().join(pid).join("comm"), format!("{command}\n")).expect("comm");
        }
        fs::create_dir_all(proc.path().join("net")).expect("create net");
        fs::write(proc.path().join("net/tcp"), TCP).expect("write the table");

        let listeners = listeners(proc.path(), Protocol::Tcp, 8080).expect("readable");
        assert_eq!(listeners.len(), 1);
        let (holders, complete) = holders(proc.path(), &[listeners[0].inode]).expect("readable");
        assert!(complete);
        assert_eq!(
            holders,
            [Holder {
                pid: 4242,
                command: "python3".into(),
                inode: 4242,
            }]
        );
    }
}
```

**Step 2: Read the journal**

journalctl explains on standard error when it shows only the caller's own messages, so the command runner also returns
standard error. In `crates/lmx-facts/src/command.rs`:

```rust
    process::{Command, Stdio},
```

with:

```rust
    process::{Command, Output, Stdio},
```

and

```rust
/// Runs `program` like [`output`], waiting at most `timeout`.
///
/// A tool that is still running is left to finish on its own instead of being killed: `systemctl`
/// gives up on D-Bus after 25 seconds, and a tool that writes after `lmx` has exited gets `SIGPIPE`.
fn output_within(program: &Path, args: &[&str], timeout: Duration) -> Result<Vec<u8>, FactError> {
```

with:

```rust
/// Runs `program` like [`output`], waiting at most `timeout`; also returns its standard error, where
/// some tools explain an incomplete answer.
pub(crate) fn output_and_errors(
    program: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<(Vec<u8>, String), FactError> {
    run(program, args, timeout).map(|output| {
        (
            output.stdout,
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    })
}

/// Runs `program` like [`output`], waiting at most `timeout`.
fn output_within(program: &Path, args: &[&str], timeout: Duration) -> Result<Vec<u8>, FactError> {
    run(program, args, timeout).map(|output| output.stdout)
}

/// Runs `program` with `args`, waiting at most `timeout`; a failure is [`FactError::Command`].
///
/// A tool that is still running is left to finish on its own instead of being killed: `systemctl`
/// gives up on D-Bus after 25 seconds, and a tool that writes after `lmx` has exited gets `SIGPIPE`.
fn run(program: &Path, args: &[&str], timeout: Duration) -> Result<Output, FactError> {
```

and

```rust
    Ok(output.stdout)
}
```

with:

```rust
    Ok(output)
}
```

Create `crates/lmx-facts/src/journal.rs`. A message that is not UTF-8 arrives as an array of bytes:

```rust
//! Records of `lmxd` tasks in the system journal, read with `journalctl -o json`.
//!
//! `lmxd` writes the output of its tasks with the fields `LMX_TASK` and `LMX_KIND`, and apply events
//! also with `LMX_GENERATION`. The journal is readable by root and the groups `wheel`, `adm` and
//! `systemd-journal`; other users see none of these records.

use std::{path::Path, time::Duration};

use serde_json::Value;

use crate::{FactError, command};

/// Longest wait for `journalctl`; a long build writes many records.
const TIMEOUT: Duration = Duration::from_secs(10);

/// One record of an `lmxd` task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Task that wrote it, such as `system-apply-3`; empty for an event outside a task.
    pub task: String,
    /// Process that wrote it; a restarted `lmxd` numbers its tasks from 1 again.
    pub pid: u32,
    /// Generation of an apply, when the record names one.
    pub generation: Option<String>,
    /// The line or event.
    pub message: String,
    /// When it was written, in microseconds since the Unix epoch.
    pub time: u64,
}

/// Which boot to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boot {
    /// The running boot.
    Current,
    /// The boot before it, such as the one that built the running generation.
    Previous,
}

/// Records of the task kind `kind` in `boot`, in the order they were written, and whether the
/// journal of the system was readable to the caller.
///
/// Only records of root count, since any user may write a record with `LMX_KIND`. A boot that the
/// journal does not have has no records.
pub fn records(
    journalctl: &Path,
    kind: &str,
    boot: Boot,
) -> Result<(Vec<Record>, bool), FactError> {
    let boot = match boot {
        Boot::Current => "-b0",
        Boot::Previous => "-b-1",
    };
    let filter = format!("LMX_KIND={kind}");
    let answer = command::output_and_errors(
        journalctl,
        &[
            "-o",
            "json",
            "--no-pager",
            "--all",
            "--output-fields=MESSAGE,LMX_TASK,LMX_GENERATION,_PID",
            boot,
            "_UID=0",
            &filter,
        ],
        TIMEOUT,
    );
    let (output, errors) = match answer {
        // journalctl fails when it can open no journal, or when the boot is not in it.
        Err(FactError::Command { stderr, .. }) if unreadable(&stderr) => {
            return Ok((Vec::new(), false));
        }
        Err(FactError::Command { stderr, .. }) if missing_boot(&stderr) => {
            return Ok((Vec::new(), true));
        }
        answer => answer?,
    };
    Ok((
        parse(&String::from_utf8_lossy(&output)),
        !unreadable(&errors),
    ))
}

/// Whether journalctl says on standard error that the caller cannot see the system's messages.
fn unreadable(errors: &str) -> bool {
    errors.contains("not seeing messages") || errors.contains("insufficient permissions")
}

/// Whether journalctl says that the journal does not have the requested boot.
fn missing_boot(errors: &str) -> bool {
    errors.contains("No journal boot entry") || errors.contains("No such boot ID")
}

/// Parses `journalctl -o json` output, one object per line; lines that are not objects are skipped.
pub fn parse(output: &str) -> Vec<Record> {
    output
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .map(|record| Record {
            task: text(&record["LMX_TASK"]).unwrap_or_default(),
            pid: text(&record["_PID"])
                .and_then(|pid| pid.parse().ok())
                .unwrap_or(0),
            generation: text(&record["LMX_GENERATION"]).filter(|generation| !generation.is_empty()),
            message: text(&record["MESSAGE"]).unwrap_or_default(),
            time: text(&record["__REALTIME_TIMESTAMP"])
                .and_then(|time| time.parse().ok())
                .unwrap_or(0),
        })
        .collect()
}

/// A field value: journalctl writes text as a string and other data as an array of bytes.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(bytes) => {
            let bytes: Option<Vec<u8>> = bytes
                .iter()
                .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
                .collect();
            Some(String::from_utf8_lossy(&bytes?).into_owned())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_task_records_with_text_and_bytes() {
        let output = concat!(
            r#"{"MESSAGE":"building the system configuration...","_PID":"812","LMX_TASK":"system-apply-3","__REALTIME_TIMESTAMP":"1791374400000000"}"#,
            "\n",
            r#"{"MESSAGE":[104,105,255],"_PID":"812","LMX_TASK":"system-apply-3","__REALTIME_TIMESTAMP":"1791374400000001"}"#,
            "\n",
            r#"{"MESSAGE":"built the generation","_PID":"812","LMX_TASK":"system-apply-3","LMX_GENERATION":"g2","__REALTIME_TIMESTAMP":"1791374400000002"}"#,
            "\n-- No entries --\n",
        );
        let records = parse(output);
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].pid, 812);
        assert_eq!(records[1].message, "hi\u{fffd}");
        assert_eq!(records[2].generation.as_deref(), Some("g2"));
        assert_eq!(records[2].time, 1_791_374_400_000_002);
    }

    #[test]
    fn tells_an_unreadable_journal_from_a_missing_boot() {
        assert!(unreadable(
            "No journal files were opened due to insufficient permissions."
        ));
        assert!(unreadable(
            "Hint: You are currently not seeing messages from other users and the system."
        ));
        assert!(missing_boot(
            "No journal boot entry found for the specified boot (-1)."
        ));
        assert!(!unreadable(
            "Failed to open the journal: Input/output error"
        ));
    }
}
```

In `crates/lmx-facts/src/lib.rs`, the table of the crate documentation:

```rust
//! | [`network`]     | interfaces and global IPv4 addresses    | `ip -j address show`                    |
```

with:

```rust
//! | [`network`]     | interfaces and global IPv4 addresses    | `ip -j address show`                    |
//! | [`sockets`]     | listening sockets and their processes   | `/proc/net`, `/proc/<pid>/fd`           |
//! | [`journal`]     | records of `lmxd` tasks                 | `journalctl -o json`                    |
```

and the modules:

```rust
pub mod generations;
pub mod machine;
```

with:

```rust
pub mod generations;
pub mod journal;
pub mod machine;
```

and

```rust
pub mod network;
pub mod units;
```

with:

```rust
pub mod network;
pub mod sockets;
pub mod units;
```

**Step 3: Run the facts tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx-facts`

Expected: 33 tests pass, among them `keeps_listening_sockets_with_their_addresses` and
`reads_task_records_with_text_and_bytes`.

---

## Task 4: Journal fields of `lmxd`

**Files:**
- Modify: `crates/lmxd/src/tasks.rs`, `crates/lmxd/src/journal.rs`, `crates/lmxd/src/apply.rs`, `crates/lmxd/src/observer.rs`, `crates/lmxd/src/main.rs`

Every task line carries `LMX_TASK` and `LMX_KIND`; apply and finalize events also carry `LMX_GENERATION`. `lmxd` starts
its logger without a field prefix, through the option of Task 1.

**Step 1: Write the failing test of a task's kind**

The journal sink knows only a task's name. In the tests of `crates/lmxd/src/tasks.rs`:

```rust
            assert_eq!(Kind::from_name(kind.name()), Some(kind));
            assert!(WorkloadTypeMeta::new(API_VERSION, kind.name()).is_ok());
        }
    }
```

with:

```rust
            assert_eq!(Kind::from_name(kind.name()), Some(kind));
            let task = format!("{}-12", kind.task_prefix());
            assert_eq!(Kind::of_task(&task), Some(kind));
            assert!(WorkloadTypeMeta::new(API_VERSION, kind.name()).is_ok());
        }
        assert_eq!(Kind::of_task("system-apply"), None);
        assert_eq!(Kind::of_task("system-apply-x"), None);
    }
```

**Step 2: Run it to see it fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmxd names_every_kind`

Expected: ``no variant or associated item named `of_task` found for enum `tasks::Kind` ``.

**Step 3: Find a task's kind and name the journal fields**

In `crates/lmxd/src/tasks.rs`, after `from_name`:

```rust
    /// Kind of the task named `task`, such as `system-apply-3`.
    pub(crate) fn of_task(task: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| {
            task.strip_prefix(kind.task_prefix())
                .and_then(|rest| rest.strip_prefix('-'))
                .is_some_and(|number| {
                    !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
                })
        })
    }
```

Replace `crates/lmxd/src/journal.rs`. A run that exited writes its exit status as a number, so `EXIT_CODE` reads as one:

```rust
//! Task output in the journal.
//!
//! Every line carries the fields `LMX_TASK` and `LMX_KIND`, such as `system-apply-3` and
//! `SystemApply`, so `lmx logs` finds the runs of a kind, also after a reboot. `lmxd` writes its
//! journal fields without a prefix.

use solti::{
    core::{TaskOutputEvent, TaskOutputSink},
    model::{OutputEvent, StreamKind},
};

use crate::tasks::Kind;

/// Logs every output line of every task, so `journalctl -u lmx` shows what the tasks printed, as
/// `journalctl -u limanix-store-guard` showed the platform guard's output.
#[derive(Debug)]
pub(crate) struct Journal;

impl TaskOutputSink for Journal {
    fn on_event(&self, event: &TaskOutputEvent) {
        let task = event.task();
        let kind = Kind::of_task(task.as_str()).map_or("", Kind::name);
        match event.event() {
            OutputEvent::Chunk(chunk) => {
                let line = String::from_utf8_lossy(&chunk.line);
                match chunk.stream {
                    StreamKind::Stdout => {
                        tracing::info!(target: "lmxd::task", lmx_task = %task, lmx_kind = kind, "{line}")
                    }
                    StreamKind::Stderr => {
                        tracing::warn!(target: "lmxd::task", lmx_task = %task, lmx_kind = kind, "{line}")
                    }
                }
            }
            OutputEvent::RunFinished {
                exit_code: Some(code),
                ..
            } => {
                tracing::info!(
                    target: "lmxd::task",
                    lmx_task = %task,
                    lmx_kind = kind,
                    exit_code = code,
                    "task run finished with exit status {code}"
                );
            }
            OutputEvent::RunFinished { .. } => {
                tracing::info!(target: "lmxd::task", lmx_task = %task, lmx_kind = kind, "task run finished");
            }
            _ => {}
        }
    }
}
```

An apply is named when it starts, so the journal has its run even when it fails before the build: the start, the build's
start and the outcome carry the task, kind and generation.

In `crates/lmxd/src/apply.rs`, replace

```rust
    generation: String,
    /// Followers and outcome.
```

with:

```rust
    generation: String,
    /// Name of the run and of its build task, such as `system-apply-3`; the journal records of the
    /// run carry it, also when it fails before the build.
    name: String,
    /// Followers and outcome.
```

and

```rust
impl Run {
    /// A run of `generation` without followers.
    fn new(generation: &str) -> Self {
        Self {
```

with:

```rust
impl Run {
    /// A run of `generation` named `name`, without followers.
    fn new(generation: &str, name: String) -> Self {
        Self {
```

and

```rust
            generation: generation.to_owned(),
            progress: SyncMutex::new(Progress {
```

with:

```rust
            generation: generation.to_owned(),
            name,
            progress: SyncMutex::new(Progress {
```

and

```rust
    gid: u32,
    /// Number of the last build task, for unique task names.
    created: AtomicU64,
```

with:

```rust
    gid: u32,
    /// Number of the last run, for unique task names.
    created: AtomicU64,
```

and

```rust
                Decision::Start => {
                    let run = Arc::new(Run::new(generation));
                    // Join before the run starts, so the follower sees its first phase.
```

with:

```rust
                Decision::Start => {
                    let number = self.created.fetch_add(1, Ordering::Relaxed) + 1;
                    let name = format!("{}-{number}", Kind::SystemApply.task_prefix());
                    let run = Arc::new(Run::new(generation, name));
                    // Join before the run starts, so the follower sees its first phase.
```

and

```rust
    async fn drive(self: Arc<Self>, run: Arc<Run>) {
        // The steps run in their own task, so even a panic gives the followers an outcome.
```

with:

```rust
    async fn drive(self: Arc<Self>, run: Arc<Run>) {
        // `lmx logs apply` reads these with the build's lines: same task, kind and generation.
        let (task, kind, generation) = (
            run.name.as_str(),
            Kind::SystemApply.name(),
            run.generation.as_str(),
        );
        tracing::info!(
            lmx_task = task,
            lmx_kind = kind,
            lmx_generation = generation,
            "applying generation {generation}"
        );
        // The steps run in their own task, so even a panic gives the followers an outcome.
```

and

```rust
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
```

with:

```rust
        match &outcome {
            Ok(_) => tracing::info!(
                lmx_task = task,
                lmx_kind = kind,
                lmx_generation = generation,
                "built generation {generation}"
            ),
            Err(error) if error.code == ErrorCode::ApplyCancelled => tracing::info!(
                lmx_task = task,
                lmx_kind = kind,
                lmx_generation = generation,
                "apply of generation {generation} cancelled"
            ),
            Err(error) => tracing::warn!(
                lmx_task = task,
                lmx_kind = kind,
                lmx_generation = generation,
                error = error.message,
                "apply of generation {generation} failed: {}",
                error.message
            ),
        }
```

and

```rust
        }
        let number = self.created.fetch_add(1, Ordering::Relaxed) + 1;
        let name = format!("{}-{number}", Kind::SystemApply.task_prefix());
        let mut lines = self.capture.listen(&name);
        // Queued: a finalize in the slot finishes first. A build of another generation was
```

with:

```rust
        }
        let name = run.name.as_str();
        let mut lines = self.capture.listen(name);
        // Queued: a finalize in the slot finishes first. A build of another generation was
```

and

```rust
        let started = match tasks::apply(&run.generation) {
            Ok(workload) => launch::start(&self.supervisor, &name, workload, placement).await,
            Err(error) => Err(error.to_string()),
```

with:

```rust
        let started = match tasks::apply(&run.generation) {
            Ok(workload) => launch::start(&self.supervisor, name, workload, placement).await,
            Err(error) => Err(error.to_string()),
```

and

```rust
        let task = started.map_err(|error| {
            self.capture.forget(&name);
            failure(
```

with:

```rust
        let task = started.map_err(|error| {
            self.capture.forget(name);
            failure(
```

and

```rust
        *run.task.lock().unwrap_or_else(PoisonError::into_inner) = Some(task.clone());
        // A cancel that came while the task was created did not see it.
```

with:

```rust
        *run.task.lock().unwrap_or_else(PoisonError::into_inner) = Some(task.clone());
        tracing::info!(
            lmx_task = name,
            lmx_kind = Kind::SystemApply.name(),
            lmx_generation = run.generation,
            "building generation {}",
            run.generation
        );
        // A cancel that came while the task was created did not see it.
```

and

```rust
        };
        self.capture.forget(&name);
        while let Ok(line) = lines.try_recv() {
```

with:

```rust
        };
        self.capture.forget(name);
        while let Ok(line) = lines.try_recv() {
```

and

```rust
        };
        let g1 = Arc::new(Run::new("g1"));
        let g2 = Arc::new(Run::new("g2"));
        let mounted_g2 = generations("g2", "g1");
```

with:

```rust
        };
        let g1 = Arc::new(Run::new("g1", "system-apply-1".into()));
        let g2 = Arc::new(Run::new("g2", "system-apply-2".into()));
        let mounted_g2 = generations("g2", "g1");
```

and

```rust
    async fn a_follower_that_joins_first_sees_every_message() {
        let run = Run::new("g1");
        let Joined::Following(mut messages) = run.join() else {
```

with:

```rust
    async fn a_follower_that_joins_first_sees_every_message() {
        let run = Run::new("g1", "system-apply-1".into());
        let Joined::Following(mut messages) = run.join() else {
```

The observer names a check or finalize before it starts, and logs a failed check, also one that failed on the mounts
before its task, under that name:

In `crates/lmxd/src/observer.rs`, replace

```rust
        let health = self.check().await;
        let healthy = health == Health::Healthy;
```

with:

```rust
        let health = self.check(&generation).await;
        let healthy = health == Health::Healthy;
```

and

```rust
        self.set_finalize(Finalize::Running);
        match self.run(Kind::SystemFinalize, tasks::finalize()).await {
            Ok(()) => {
```

with:

```rust
        self.set_finalize(Finalize::Running);
        let task = self.name(Kind::SystemFinalize);
        let kind = Kind::SystemFinalize.name();
        match self
            .run(&task, Kind::SystemFinalize, tasks::finalize())
            .await
        {
            Ok(()) => {
```

and

```rust
            Ok(()) => {
                tracing::info!(generation, "finalized the booted generation");
                self.set_finalize(Finalize::Idle);
```

with:

```rust
            Ok(()) => {
                tracing::info!(
                    lmx_task = task,
                    lmx_kind = kind,
                    lmx_generation = generation,
                    "finalized generation {generation}"
                );
                self.set_finalize(Finalize::Idle);
```

and

```rust
                tracing::warn!(
                    generation,
                    reason,
```

with:

```rust
                tracing::warn!(
                    lmx_task = task,
                    lmx_kind = kind,
                    lmx_generation = generation,
                    reason,
```

and

```rust
                    reason,
                    "finalizing the booted generation failed"
                );
```

with:

```rust
                    reason,
                    "finalizing generation {generation} failed: {reason}"
                );
```

and

```rust
    /// Checks the booted generation: the mounts in process, then the health task.
    async fn check(&self) -> Health {
        if let Err(reason) = self.mounts() {
            return Health::Unhealthy(reason);
        }
```

with:

```rust
    /// Checks the booted `generation`: the mounts in process, then the health task.
    ///
    /// A failed check is logged under the check's task name, also when it failed before the task.
    async fn check(&self, generation: &str) -> Health {
        let task = self.name(Kind::SystemHealth);
        let health = match self.mounts() {
            Err(reason) => Health::Unhealthy(reason),
            Ok(()) => match self.run(&task, Kind::SystemHealth, tasks::health()).await {
                Ok(()) => Health::Healthy,
                Err(reason) => Health::Unhealthy(reason),
            },
        };
        if let Health::Unhealthy(reason) = &health {
            tracing::warn!(
                lmx_task = task,
                lmx_kind = Kind::SystemHealth.name(),
                lmx_generation = generation,
                "generation {generation} is unhealthy: {reason}"
            );
        }
```

and

```rust
        }
        match self.run(Kind::SystemHealth, tasks::health()).await {
            Ok(()) => Health::Healthy,
            Err(reason) => Health::Unhealthy(reason),
        }
    }
```

with:

```rust
        }
        health
    }

    /// A new task name of `kind`, such as `system-health-3`.
    fn name(&self, kind: Kind) -> String {
        let number = self.created.fetch_add(1, Ordering::Relaxed) + 1;
        format!("{}-{number}", kind.task_prefix())
    }
```

and

```rust
    /// Runs a task of `kind` once and waits; a failure gives the last line it printed, or why it
    /// ended.
    async fn run(&self, kind: Kind, workload: ModelResult<TaskWorkload>) -> Result<(), String> {
        let number = self.created.fetch_add(1, Ordering::Relaxed) + 1;
        let name = format!("{}-{number}", kind.task_prefix());
        let mut lines = self.capture.listen(&name);
        // A finalize never waits behind a build in the system slot: after the build, the older
```

with:

```rust
    /// Runs the task `name` of `kind` once and waits; a failure gives the last line it printed, or
    /// why it ended.
    async fn run(
        &self,
        name: &str,
        kind: Kind,
        workload: ModelResult<TaskWorkload>,
    ) -> Result<(), String> {
        let mut lines = self.capture.listen(name);
        // A finalize never waits behind a build in the system slot: after the build, the older
```

and

```rust
        let started = match workload {
            Ok(workload) => launch::start(&self.supervisor, &name, workload, placement).await,
            Err(error) => Err(error.to_string()),
```

with:

```rust
        let started = match workload {
            Ok(workload) => launch::start(&self.supervisor, name, workload, placement).await,
            Err(error) => Err(error.to_string()),
```

and

```rust
            Err(error) => {
                self.capture.forget(&name);
                return Err(error);
```

with:

```rust
            Err(error) => {
                self.capture.forget(name);
                return Err(error);
```

and

```rust
        };
        self.capture.forget(&name);
        let mut last = None;
```

with:

```rust
        };
        self.capture.forget(name);
        let mut last = None;
```

In `crates/lmxd/src/main.rs`, the logger:

```rust
        LoggerFormat::Text
    };
    if let Err(error) = init_logger(&LoggerConfig {
        format,
        ..LoggerConfig::default()
    }) {
```

with:

```rust
        LoggerFormat::Text
    };
    // Journal fields keep their names, such as `LMX_TASK`, so `lmx logs` and people filter by them.
    if let Err(error) = init_logger(&LoggerConfig {
        format,
        journald_field_prefix: None,
        ..LoggerConfig::default()
    }) {
```

**Step 4: Run the daemon tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmxd`

Expected: 26 tests pass.

---

## Task 5: Colors from the declared theme

**Files:**
- Modify: `crates/lmx/src/palette.rs`, `crates/lmx/src/welcome.rs`

`lmx` stops hard-coding Catppuccin Mocha: the palette comes from `theme.palette` in the configuration, color by color,
and Mocha remains the fallback for a missing or invalid color and for a missing configuration.

**Step 1: Read the palette from the configuration**

Replace `crates/lmx/src/palette.rs`. `Paint` carries the palette, so code that colors text asks `paint.palette()`; its
test takes a theme with a valid, an invalid and a missing color:

```rust
//! Colors of the guest's text: the theme of the declaration, Catppuccin Mocha without one.

use std::{
    env,
    io::{self, IsTerminal},
};

use lmx_model::{Config, Theme};

/// One palette color as 24-bit RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Color(u8, u8, u8);

impl Color {
    /// Parses `#rrggbb`.
    fn parse(text: &str) -> Option<Self> {
        let hex = text
            .strip_prefix('#')
            .filter(|hex| hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))?;
        let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
        Some(Self(channel(0)?, channel(2)?, channel(4)?))
    }
}

/// The colors `lmx` uses, by the theme's color names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Palette {
    /// `blue`: commands and the logo.
    pub(crate) blue: Color,
    /// `mauve`: the second half of the logo.
    pub(crate) mauve: Color,
    /// `subtext0`: secondary values.
    pub(crate) subtext: Color,
    /// `overlay1`: labels.
    pub(crate) muted: Color,
    /// `green`: writable shared folders.
    pub(crate) green: Color,
    /// `peach`: read-only shared folders.
    pub(crate) peach: Color,
    /// `yellow`: warnings.
    pub(crate) yellow: Color,
}

impl Palette {
    /// Catppuccin Mocha, the platform's palette until the declaration chooses a theme.
    pub(crate) const MOCHA: Self = Self {
        blue: Color(137, 180, 250),
        mauve: Color(203, 166, 247),
        subtext: Color(166, 173, 200),
        muted: Color(127, 132, 156),
        green: Color(166, 227, 161),
        peach: Color(250, 179, 135),
        yellow: Color(249, 226, 175),
    };

    /// The palette of `theme`; a color it lacks, or one that is not `#rrggbb`, stays Mocha's.
    pub(crate) fn of(theme: &Theme) -> Self {
        let pick = |name: &str, mocha: Color| {
            theme
                .palette
                .get(name)
                .and_then(|value| Color::parse(value))
                .unwrap_or(mocha)
        };
        let mocha = Self::MOCHA;
        Self {
            blue: pick("blue", mocha.blue),
            mauve: pick("mauve", mocha.mauve),
            subtext: pick("subtext0", mocha.subtext),
            muted: pick("overlay1", mocha.muted),
            green: pick("green", mocha.green),
            peach: pick("peach", mocha.peach),
            yellow: pick("yellow", mocha.yellow),
        }
    }

    /// The palette of the configuration, or Mocha without one.
    pub(crate) fn of_config(config: &Result<Config, String>) -> Self {
        config
            .as_ref()
            .map_or(Self::MOCHA, |config| Self::of(&config.theme))
    }
}

/// Whether and how text is colored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Paint {
    /// `true` when escape sequences are written.
    enabled: bool,
    /// The colors.
    palette: Palette,
}

impl Paint {
    /// Colors with `palette` only a terminal that is not `dumb`, and never when `NO_COLOR` is set;
    /// an empty `TERM` counts as `dumb`, as in the platform prompt.
    pub(crate) fn detect(palette: Palette) -> Self {
        let terminal = io::stdout().is_terminal();
        let no_color = env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
        let dumb = env::var_os("TERM").is_none_or(|term| term.is_empty() || term == "dumb");
        Self {
            enabled: terminal && !no_color && !dumb,
            palette,
        }
    }

    /// Plain text, for tests.
    #[cfg(test)]
    pub(crate) const fn plain() -> Self {
        Self {
            enabled: false,
            palette: Palette::MOCHA,
        }
    }

    /// Text colored with Mocha, for tests.
    #[cfg(test)]
    pub(crate) const fn colored() -> Self {
        Self {
            enabled: true,
            palette: Palette::MOCHA,
        }
    }

    /// The colors.
    pub(crate) const fn palette(self) -> Palette {
        self.palette
    }

    /// `text` in `color`.
    pub(crate) fn color(self, color: Color, text: &str) -> String {
        if self.enabled && !text.is_empty() {
            let Color(red, green, blue) = color;
            format!("\x1b[38;2;{red};{green};{blue}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    /// `text` in bold.
    pub(crate) fn bold(self, text: &str) -> String {
        if self.enabled && !text.is_empty() {
            format!("\x1b[1m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_has_no_escapes() {
        assert_eq!(Paint::plain().color(Palette::MOCHA.blue, "lmx"), "lmx");
        assert_eq!(Paint::plain().bold("dev-box"), "dev-box");
    }

    #[test]
    fn colored_text_uses_true_color() {
        assert_eq!(
            Paint::colored().color(Palette::MOCHA.blue, "lmx"),
            "\x1b[38;2;137;180;250mlmx\x1b[0m"
        );
        assert_eq!(Paint::colored().bold("x"), "\x1b[1mx\x1b[0m");
        assert_eq!(Paint::colored().color(Palette::MOCHA.yellow, ""), "");
    }

    #[test]
    fn takes_the_themes_colors_and_keeps_mocha_for_the_rest() {
        let theme = Theme {
            flavor: "latte".into(),
            palette: [
                ("blue", "#1e66f5"),
                ("peach", "not a color"),
                ("green", "#40A02B"),
            ]
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
        };
        let palette = Palette::of(&theme);
        assert_eq!(palette.blue, Color(30, 102, 245));
        assert_eq!(palette.green, Color(64, 160, 43));
        assert_eq!(palette.peach, Palette::MOCHA.peach);
        assert_eq!(palette.mauve, Palette::MOCHA.mauve);
        assert_eq!(
            Palette::of_config(&Err("unreadable".into())),
            Palette::MOCHA
        );
    }
}
```

**Step 2: Color the welcome with it**

In `crates/lmx/src/welcome.rs`, replace

```rust
    output,
    palette::{self, Color, Paint},
    system::System,
```

with:

```rust
    output,
    palette::{Color, Paint, Palette},
    system::System,
```

and

```rust
    let facts = collect(system);
    output::write_text(&render(&facts, Paint::detect()))?;
    Ok(output::page_status(facts.mounts.is_some()))
```

with:

```rust
    let facts = collect(system);
    output::write_text(&render(
        &facts,
        Paint::detect(Palette::of_config(&system.config)),
    ))?;
    Ok(output::page_status(facts.mounts.is_some()))
```

and

```rust
            "  {}{}\n",
            paint.color(palette::BLUE, left),
            paint.color(palette::MAUVE, right)
        ));
```

with:

```rust
            "  {}{}\n",
            paint.color(paint.palette().blue, left),
            paint.color(paint.palette().mauve, right)
        ));
```

and

```rust
        "  {}{}{}{}{}{}\n\n",
        paint.color(palette::BLUE, "lmx help"),
        paint.color(palette::MUTED, " for commands, "),
        paint.color(palette::BLUE, "lmx info"),
        paint.color(palette::MUTED, " for details, "),
        paint.color(palette::BLUE, "exit"),
        paint.color(palette::MUTED, " to return to the Mac.")
    ));
```

with:

```rust
        "  {}{}{}{}{}{}\n\n",
        paint.color(paint.palette().blue, "lmx help"),
        paint.color(paint.palette().muted, " for commands, "),
        paint.color(paint.palette().blue, "lmx info"),
        paint.color(paint.palette().muted, " for details, "),
        paint.color(paint.palette().blue, "exit"),
        paint.color(paint.palette().muted, " to return to the Mac.")
    ));
```

and

```rust
fn label(paint: Paint, label: &str) -> String {
    paint.color(palette::MUTED, &format!("{label:<LABEL$}"))
}
```

with:

```rust
fn label(paint: Paint, label: &str) -> String {
    paint.color(paint.palette().muted, &format!("{label:<LABEL$}"))
}
```

and

```rust
            paint.bold(&facts.name),
            paint.color(palette::SUBTEXT, &system)
        ));
```

with:

```rust
            paint.bold(&facts.name),
            paint.color(paint.palette().subtext, &system)
        ));
```

and

```rust
        row(text, paint, "VM", &facts.name, None);
        row(text, paint, "", &system, Some(palette::SUBTEXT));
    }
```

with:

```rust
        row(text, paint, "VM", &facts.name, None);
        row(text, paint, "", &system, Some(paint.palette().subtext));
    }
```

and

```rust
        let (mode, color) = if mount.read_only {
            ("ro", palette::PEACH)
        } else {
```

with:

```rust
        let (mode, color) = if mount.read_only {
            ("ro", paint.palette().peach)
        } else {
```

and

```rust
        } else {
            ("rw", palette::GREEN)
        };
```

with:

```rust
        } else {
            ("rw", paint.palette().green)
        };
```

and

```rust
    for line in wrap(warning, WARNING) {
        text.push_str(&format!("  {}\n", paint.color(palette::YELLOW, &line)));
    }
```

with:

```rust
    for line in wrap(warning, WARNING) {
        text.push_str(&format!(
            "  {}\n",
            paint.color(paint.palette().yellow, &line)
        ));
    }
```

**Step 3: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx --bins`

Expected: 40 tests pass.

---

## Task 6: `lmx doctor` and `lmx status --short`

**Files:**
- Create: `crates/lmx/src/findings.rs`, `crates/lmx/src/doctor.rs`
- Modify: `crates/lmx/src/palette.rs`, `crates/lmx/src/owner.rs`, `crates/lmx/src/status.rs`, `crates/lmx/src/cli.rs`, `crates/lmx/src/main.rs`, `crates/lmx/src/help.rs`
- Test: `crates/lmx/tests/cli.rs`, `crates/lmx/tests/owner.rs`

Both commands read the conditions of `lmxd`. `doctor` turns them into check records with what to do next and works
without `lmxd`, reading the generation markers instead. `--short` prints only the words that need attention and gives
`lmxd` 200 ms.

**Step 1: Write the failing tests**

At the end of `crates/lmx/tests/cli.rs`, without `lmxd`: `owner` fails, the markers give a warning, and `--short` says
`lmxd?`:

```rust
#[test]
fn doctor_without_lmxd_reports_the_owner_and_reads_the_markers() {
    let guest = Guest::new();
    let output = guest.lmx(&["doctor", "--json"], &guest.config());
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let answer: Value = serde_json::from_slice(&output.stdout).expect("one JSON answer");
    let checks: Vec<(&str, &str)> = answer["data"]["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .map(|check| {
            (
                check["check"].as_str().unwrap_or_default(),
                check["status"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        checks,
        [
            ("config", "ok"),
            ("owner", "failed"),
            ("generations", "warning")
        ]
    );

    let short = guest.lmx(&["status", "--short"], &guest.config());
    assert!(short.status.success(), "{short:?}");
    assert_eq!(String::from_utf8_lossy(&short.stdout), "lmxd?\n");
}
```

At the end of `crates/lmx/tests/owner.rs`, with `lmxd` and a built generation that waits for a restart: a warning does
not fail `doctor`, and `--short` says `restart`:

```rust
#[test]
fn doctor_and_the_short_status_follow_the_conditions_of_lmxd() {
    let guest = Guest::new(usage(50), usage(50));
    guest.mount("g2", "g2", "g1", 1);
    let output = guest.lmx(&["doctor", "--json"]).output().expect("run lmx");
    assert!(output.status.success(), "warnings do not fail: {output:?}");
    let checks = &answer(&output)["data"]["checks"];
    assert_eq!(checks[1]["status"], "ok", "{checks}");
    assert_eq!(
        checks[2],
        json!({
            "check": "generations",
            "status": "warning",
            "message": "Generation g2 is built; restart the VM to boot it.",
            "hint": "Restart the VM from the Mac; limanix update does it."
        })
    );

    let short = guest.lmx(&["status", "--short"]).output().expect("run lmx");
    assert!(short.status.success(), "{short:?}");
    assert_eq!(String::from_utf8_lossy(&short.stdout), "restart\n");
}
```

**Step 2: Run them to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx --test cli --test owner --no-fail-fast`

Expected: the new tests fail with ``error: unrecognized subcommand 'doctor'``.

**Step 3: Color failures**

In `crates/lmx/src/palette.rs`, the palette gains `red`:

```rust
    /// `green`: writable shared folders.
```

with:

```rust
    /// `green`: passed checks and writable shared folders.
```

and

```rust
    pub(crate) yellow: Color,
}
```

with:

```rust
    pub(crate) yellow: Color,
    /// `red`: failed checks.
    pub(crate) red: Color,
}
```

and

```rust
        yellow: Color(249, 226, 175),
    };
```

with:

```rust
        yellow: Color(249, 226, 175),
        red: Color(243, 139, 168),
    };
```

and

```rust
            yellow: pick("yellow", mocha.yellow),
        }
```

with:

```rust
            yellow: pick("yellow", mocha.yellow),
            red: pick("red", mocha.red),
        }
```

**Step 4: Render check records**

Create `crates/lmx/src/findings.rs`, shared with `net check` in Task 7. The status is padded before it is colored, so
the columns line up on a terminal too:

```rust
//! Check records of `lmx doctor` and `lmx net check`, for people and for the host.

use std::{io, process::ExitCode};

use lmx_model::{Check, CheckStatus};

use crate::{output, palette::Paint};

/// Width of the status column.
const STATUS: usize = 8;

/// Width of the check column.
const CHECK: usize = 11;

/// Name of a status for people.
const fn name(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Ok => "ok",
        CheckStatus::Warning => "warning",
        CheckStatus::Failed => "failed",
        CheckStatus::Unknown => "unknown",
    }
}

/// Renders `checks` as aligned rows, each hint on its own line under the message.
pub(crate) fn render(checks: &[Check], paint: Paint) -> String {
    let palette = paint.palette();
    let mut text = String::new();
    for check in checks {
        let color = match check.status {
            CheckStatus::Ok => palette.green,
            CheckStatus::Warning => palette.yellow,
            CheckStatus::Failed => palette.red,
            CheckStatus::Unknown => palette.muted,
        };
        let status = format!("{:<STATUS$}", name(check.status));
        text.push_str(&format!(
            "{} {:<CHECK$} {}\n",
            paint.color(color, &status),
            check.check,
            check.message
        ));
        if let Some(hint) = &check.hint {
            text.push_str(&format!(
                "{:indent$}{}\n",
                "",
                paint.color(palette.muted, hint),
                indent = STATUS + CHECK + 2
            ));
        }
    }
    text
}

/// Writes `checks` for people and exits with failure when any check failed.
pub(crate) fn write(checks: &[Check], paint: Paint) -> io::Result<ExitCode> {
    output::write_text(&render(checks, paint))?;
    Ok(status(checks))
}

/// Exit status of an answer with `checks`: a failed check fails the command, a warning does not.
pub(crate) fn status(checks: &[Check]) -> ExitCode {
    output::page_status(
        !checks
            .iter()
            .any(|check| check.status == CheckStatus::Failed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_checks_and_puts_hints_under_the_message() {
        let checks = [
            Check::new("owner", CheckStatus::Ok, "lmxd 0.1.0 answers."),
            Check::new("firewall", CheckStatus::Failed, "TCP 8080 is not open.")
                .hint("Add it to network.ports."),
        ];
        assert_eq!(
            render(&checks, Paint::plain()),
            "ok       owner       lmxd 0.1.0 answers.\n\
             failed   firewall    TCP 8080 is not open.\n\
             \x20                    Add it to network.ports.\n"
        );
    }
}
```

**Step 5: Diagnose**

Create `crates/lmx/src/doctor.rs`. The decisions are pure functions of the answer of `lmxd` and the markers, with table
tests:

```rust
//! `lmx doctor`: what is wrong with the guest owner, and what to do about it.
//!
//! It works without `lmxd`, because diagnosing `lmxd` is one of its jobs. Each check is a record with
//! a status and, when there is something to do, a hint; `--json` answers with the records.

use std::{io, process::ExitCode};

use lmx_facts::{FactError, generations};
use lmx_model::{
    CONFIG_PATH, CONVERGED, Check, CheckStatus, DEGRADED, DISK_LOW, Doctor, Envelope, Generations,
    OUT_OF_DATE, Owner, RESTART_REQUIRED,
};

use crate::{
    cli::OutputArgs,
    findings, output,
    owner::{self, CallError},
    palette::{Paint, Palette},
    system::System,
};

/// Hint of an `lmxd` that does not answer.
const OWNER_HINT: &str = "Check systemctl status lmx.socket lmx.service and journalctl -u lmx.";

/// Runs `lmx doctor`.
pub(crate) fn run(system: &System, args: &OutputArgs) -> io::Result<ExitCode> {
    let owner = owner::status(&system.owner_socket());
    let (markers, errors) = generations::read(&system.generation_paths());
    let mut checks = vec![config(system), owner_check(&owner)];
    checks.extend(conditions(owner.as_ref().ok(), &markers, errors.first()));
    if args.json {
        output::write_json(&Envelope::success(Doctor {
            checks: checks.clone(),
        }))?;
        return Ok(findings::status(&checks));
    }
    findings::write(&checks, Paint::detect(Palette::of_config(&system.config)))
}

/// Whether the configuration is readable and valid.
fn config(system: &System) -> Check {
    match &system.config {
        Ok(config) => Check::new(
            "config",
            CheckStatus::Ok,
            format!("{CONFIG_PATH} is valid; generation {}.", config.generation),
        ),
        Err(message) => Check::new("config", CheckStatus::Failed, message.clone())
            .hint("The platform renders it; limanix update on the Mac restores it."),
    }
}

/// Whether `lmxd` answers, and with the version of `lmx`.
fn owner_check(answer: &Result<Owner, CallError>) -> Check {
    let version = env!("CARGO_PKG_VERSION");
    match answer {
        Ok(owner) if owner.version == version => Check::new(
            "owner",
            CheckStatus::Ok,
            format!("lmxd {} answers.", owner.version),
        ),
        Ok(owner) => Check::new(
            "owner",
            CheckStatus::Warning,
            format!("lmxd {} answers, but lmx is {version}.", owner.version),
        )
        .hint("Restart the VM so both come from the booted generation."),
        Err(error) => Check::new("owner", CheckStatus::Failed, error.to_string()).hint(OWNER_HINT),
    }
}

/// The conditions of `lmxd`, or what the generation markers say without it; `error` is the first
/// marker that exists but could not be read.
fn conditions(
    owner: Option<&Owner>,
    markers: &Generations,
    error: Option<&FactError>,
) -> Vec<Check> {
    let Some(owner) = owner else {
        return vec![from_markers(markers, error)];
    };
    let holds = |kind: &str| {
        owner
            .conditions
            .iter()
            .find(|condition| condition.kind == kind)
            .map(|condition| condition.message.clone())
    };
    let applying = owner
        .operations
        .iter()
        .any(|operation| operation.kind == "SystemApply");
    let generation = if let Some(message) = holds(DEGRADED) {
        Check::new("generations", CheckStatus::Failed, message).hint(
            "See sudo lmx logs health, and the unit the reason names with systemctl status; lmxd \
             checks again every minute.",
        )
    } else if let Some(message) = holds(RESTART_REQUIRED) {
        Check::new("generations", CheckStatus::Warning, message)
            .hint("Restart the VM from the Mac; limanix update does it.")
    } else if let Some(message) = holds(OUT_OF_DATE) {
        let hint = if applying {
            "An update is in progress; wait for limanix update to finish."
        } else {
            "The host builds it: run limanix update on the Mac."
        };
        Check::new("generations", CheckStatus::Warning, message).hint(hint)
    } else if let Some(message) = holds(CONVERGED) {
        Check::new("generations", CheckStatus::Ok, message)
    } else {
        settled(markers)
    };
    let mut checks = vec![generation];
    if let Some(message) = holds(DISK_LOW) {
        checks.push(
            Check::new("disk", CheckStatus::Warning, message)
                .hint("Free space with sudo lmx store reserve."),
        );
    }
    checks
}

/// What the generation markers say when `lmxd` does not answer.
///
/// A marker the caller may not read, such as the mounted one for people, makes the check `unknown`;
/// any other read error fails it.
fn from_markers(markers: &Generations, error: Option<&FactError>) -> Check {
    match error {
        Some(FactError::Io { source, .. }) if source.kind() == io::ErrorKind::PermissionDenied => {
            return Check::new(
                "generations",
                CheckStatus::Unknown,
                "The mounted generation is readable only by root.",
            )
            .hint("Run sudo lmx doctor.");
        }
        Some(error) => {
            return Check::new("generations", CheckStatus::Failed, error.to_string());
        }
        None => {}
    }
    match (&markers.desired, &markers.built, &markers.booted) {
        (Some(desired), built, _) if built.as_ref() != Some(desired) => Check::new(
            "generations",
            CheckStatus::Warning,
            format!("Generation {desired} is mounted but not built; apply it."),
        )
        .hint("The host builds it: run limanix update on the Mac."),
        (Some(desired), _, booted) if booted.as_ref() != Some(desired) => Check::new(
            "generations",
            CheckStatus::Warning,
            format!("Generation {desired} is built; restart the VM to boot it."),
        )
        .hint("Restart the VM from the Mac; limanix update does it."),
        _ => settled(markers),
    }
}

/// A generation without a condition: booted and finalizing, or a system without markers.
fn settled(markers: &Generations) -> Check {
    match &markers.booted {
        Some(booted) => Check::new(
            "generations",
            CheckStatus::Ok,
            format!("Generation {booted} is booted."),
        ),
        None => Check::new(
            "generations",
            CheckStatus::Ok,
            "The system has no generation markers; it was built before lmx.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use lmx_model::{Condition, Operation};

    use super::*;

    /// An answer of `lmxd` with conditions of `kinds` and, when `applying`, a running apply.
    fn owner(kinds: &[&str], applying: bool) -> Owner {
        Owner {
            version: env!("CARGO_PKG_VERSION").into(),
            conditions: kinds
                .iter()
                .map(|kind| Condition {
                    kind: (*kind).into(),
                    message: format!("{kind} message"),
                })
                .collect(),
            operations: applying
                .then(|| Operation {
                    task: "system-apply-1".into(),
                    kind: "SystemApply".into(),
                    phase: "running".into(),
                    created_at: 0,
                })
                .into_iter()
                .collect(),
        }
    }

    /// Markers of three stages.
    fn markers(desired: &str, built: &str, booted: &str) -> Generations {
        let stage = |value: &str| (!value.is_empty()).then(|| value.to_owned());
        Generations {
            desired: stage(desired),
            built: stage(built),
            booted: stage(booted),
        }
    }

    /// Statuses and names of `checks`.
    fn summary(checks: &[Check]) -> Vec<(CheckStatus, &str)> {
        checks
            .iter()
            .map(|check| (check.status, check.check.as_str()))
            .collect()
    }

    #[test]
    fn turns_the_conditions_of_lmxd_into_findings() {
        let settled = markers("g1", "g1", "g1");
        let checks = conditions(Some(&owner(&[DEGRADED, DISK_LOW], false)), &settled, None);
        assert_eq!(
            summary(&checks),
            [
                (CheckStatus::Failed, "generations"),
                (CheckStatus::Warning, "disk")
            ]
        );
        let checks = conditions(Some(&owner(&[OUT_OF_DATE], true)), &settled, None);
        assert_eq!(
            checks[0].hint.as_deref(),
            Some("An update is in progress; wait for limanix update to finish.")
        );
        let checks = conditions(Some(&owner(&[], false)), &settled, None);
        assert_eq!(checks[0].message, "Generation g1 is booted.");
    }

    #[test]
    fn reads_the_markers_without_lmxd() {
        let check = |desired, built, booted| {
            conditions(None, &markers(desired, built, booted), None)[0].status
        };
        assert_eq!(check("g2", "g1", "g1"), CheckStatus::Warning);
        assert_eq!(check("g2", "g2", "g1"), CheckStatus::Warning);
        assert_eq!(check("g2", "g2", "g2"), CheckStatus::Ok);
        assert_eq!(check("", "", ""), CheckStatus::Ok);
        let error = |kind| FactError::Io {
            what: "the desired generation",
            source: io::Error::from(kind),
        };
        let denied = error(io::ErrorKind::PermissionDenied);
        let unreadable = conditions(None, &markers("", "g1", "g1"), Some(&denied));
        assert_eq!(unreadable[0].status, CheckStatus::Unknown);
        let broken = error(io::ErrorKind::NotConnected);
        let failed = conditions(None, &markers("", "g1", "g1"), Some(&broken));
        assert_eq!(failed[0].status, CheckStatus::Failed);
    }

    #[test]
    fn a_daemon_of_another_version_is_a_warning() {
        let mut other = owner(&[], false);
        other.version = "0.0.1".into();
        assert_eq!(owner_check(&Ok(other)).status, CheckStatus::Warning);
        let unreachable = CallError::Unavailable("lmxd is not reachable".into());
        assert_eq!(owner_check(&Err(unreachable)).status, CheckStatus::Failed);
    }
}
```

**Step 6: Ask `lmxd` with a timeout of the caller's choice**

In `crates/lmx/src/owner.rs`:

```rust
/// Asks `lmxd` on `socket` for its state, waiting at most [`STATUS_TIMEOUT`].
pub(crate) fn status(socket: &Path) -> Result<Owner, CallError> {
    status_within(socket, STATUS_TIMEOUT)
}

/// Asks `lmxd` on `socket` for its state, waiting at most `timeout` to connect and get the answer.
pub(crate) fn status_within(socket: &Path, timeout: Duration) -> Result<Owner, CallError> {
    block_on(async {
        let call = async {
            let mut client = connect(socket).await?;
            let response = client
                .status(StatusRequest {})
                .await
                .map_err(|status| CallError::from_status(&status))?;
            Ok(Owner::from(response.into_inner()))
        };
        tokio::time::timeout(timeout, call)
            .await
            .unwrap_or_else(|_| Err(silent(timeout)))
    })
}
```

and the message of a silent daemon names milliseconds when the timeout has them:

```rust
/// Error of a daemon that did not answer within `timeout`.
fn silent(timeout: Duration) -> CallError {
    CallError::Unavailable(format!(
        "lmxd did not answer within {} seconds",
        timeout.as_secs()
    ))
}
```

with:

```rust
/// Error of a daemon that did not answer within `timeout`.
fn silent(timeout: Duration) -> CallError {
    let span = if timeout.subsec_millis() == 0 {
        format!("{} seconds", timeout.as_secs())
    } else {
        format!("{} ms", timeout.as_millis())
    };
    CallError::Unavailable(format!("lmxd did not answer within {span}"))
}
```

**Step 7: Print the short status**

In `crates/lmx/src/status.rs`, the module documentation:

```rust
//! `lmx status`: what the guest is right now.
//!
//! Every fact is read independently. A fact that cannot be read becomes `null` with a
//! [`Problem`], so the host and people always get the rest. The owner part comes from `lmxd` when it
//! answers within two seconds.
```

with:

```rust
//! `lmx status`: what the guest is right now.
//!
//! Every fact is read independently. A fact that cannot be read becomes `null` with a
//! [`Problem`], so the host and people always get the rest. The owner part comes from `lmxd` when it
//! answers within two seconds. `--short` asks `lmxd` only, and prints what needs attention.
```

the imports and the start of `run`:

```rust
use std::{io, process::ExitCode, time::Duration};

use lmx_facts::{FactError, disk, generations, network, units};
use lmx_model::{
    DEGRADED, DISK_LOW, Envelope, OUT_OF_DATE, Owner, Problem, RESTART_REQUIRED, Status,
};

use crate::{
    cli::{Goal, StatusArgs},
    format, layout, output, owner,
    system::System,
    wait,
};

/// How long `lmx status --short` waits for `lmxd`; a prompt must never hang.
const SHORT_TIMEOUT: Duration = Duration::from_millis(200);

/// Runs `lmx status`, waits for a goal first with `--wait`, or prints the short form.
pub(crate) fn run(system: &System, args: &StatusArgs) -> io::Result<ExitCode> {
    if args.short {
        let words = owner::status_within(&system.owner_socket(), SHORT_TIMEOUT)
            .map_or_else(|_| vec!["lmxd?"], |owner| short(&owner));
        if !words.is_empty() {
            output::write_text(&format!("{}\n", words.join(" ")))?;
        }
        return Ok(ExitCode::SUCCESS);
    }
```

the words, before `collect`:

```rust
/// The words of `lmx status --short`: what needs attention, most urgent first; none when all is
/// well.
fn short(owner: &Owner) -> Vec<&'static str> {
    let holds = |kind: &str| {
        owner
            .conditions
            .iter()
            .any(|condition| condition.kind == kind)
    };
    let applying = owner
        .operations
        .iter()
        .any(|operation| operation.kind == "SystemApply");
    let mut words = Vec::new();
    if holds(DEGRADED) {
        words.push("degraded");
    }
    if holds(DISK_LOW) {
        words.push("disk-low");
    }
    if holds(RESTART_REQUIRED) {
        words.push("restart");
    }
    if applying {
        words.push("applying");
    } else if holds(OUT_OF_DATE) {
        words.push("apply");
    }
    words
}
```

and their test:

```rust
    #[test]
    fn the_short_form_names_only_what_needs_attention() {
        let owner = |kinds: &[&str], applying: bool| Owner {
            version: "0.1.0".into(),
            conditions: kinds
                .iter()
                .map(|kind| Condition {
                    kind: (*kind).into(),
                    message: String::new(),
                })
                .collect(),
            operations: applying
                .then(|| Operation {
                    task: "system-apply-1".into(),
                    kind: "SystemApply".into(),
                    phase: "running".into(),
                    created_at: 0,
                })
                .into_iter()
                .collect(),
        };
        assert!(short(&owner(&["Converged"], false)).is_empty());
        assert_eq!(
            short(&owner(&[RESTART_REQUIRED, DISK_LOW, DEGRADED], false)),
            ["degraded", "disk-low", "restart"]
        );
        assert_eq!(short(&owner(&[OUT_OF_DATE], false)), ["apply"]);
        assert_eq!(short(&owner(&[OUT_OF_DATE], true)), ["applying"]);
    }
```

**Step 8: Add the commands**

In `crates/lmx/src/cli.rs`, the command:

```rust
    Status(StatusArgs),
```

with:

```rust
    Status(StatusArgs),
    /// Diagnose the configuration, lmxd and the generations, with what to do next.
    Doctor(OutputArgs),
```

the flag:

```rust
pub(crate) struct StatusArgs {
```

with:

```rust
pub(crate) struct StatusArgs {
    /// Print only what needs attention, such as `restart`, for tmux and the prompt.
    #[arg(long, conflicts_with_all = ["wait", "json"])]
    pub(crate) short: bool,
```

and its test, at the end of the tests:

```rust
    #[test]
    fn the_short_status_takes_neither_json_nor_a_wait() {
        assert!(Cli::try_parse_from(["lmx", "status", "--short"]).is_ok());
        assert!(Cli::try_parse_from(["lmx", "status", "--short", "--json"]).is_err());
        let wait = [
            "lmx",
            "status",
            "--short",
            "--wait",
            "converged",
            "-g",
            "g1",
        ];
        assert!(Cli::try_parse_from(wait).is_err());
    }
```

In `crates/lmx/src/main.rs`, the crate documentation:

```rust
//! | `lmx status`          | facts  | generations, disk, interfaces, failed units, and `lmxd`     |
```

with:

```rust
//! | `lmx status`          | facts  | generations, disk, interfaces, failed units, and `lmxd`     |
//! | `lmx doctor`          | facts  | findings about the configuration, `lmxd` and generations    |
```

and

```rust
//! settled after a restart. Caller commands act on the caller's terminal and environment, so only
//! the caller can run them.
```

with:

```rust
//! settled after a restart. `lmx status --short` prints only what needs attention, for tmux and the
//! prompt, and never waits more than 200 ms. Caller commands act on the caller's terminal and
//! environment, so only the caller can run them.
```

the modules:

```rust
mod clipboard;
mod format;
```

with:

```rust
mod clipboard;
mod doctor;
mod findings;
mod format;
```

and the dispatch:

```rust
        Some(Command::Version(args)) => version::run(&args),
```

with:

```rust
        Some(Command::Version(args)) => version::run(&args),
        Some(Command::Doctor(args)) => doctor::run(&System::from_environment(), &args),
```

In `crates/lmx/src/help.rs`, the guest page lists `lmx doctor`:

```rust
  lmx status            Show generations, disk, network, failed units; sudo shows every fact.
```

with:

```rust
  lmx status            Show generations, disk, network, failed units; sudo shows every fact.
  lmx doctor            Find what is wrong with the guest owner and what to do about it.
```

and its test expects it:

```rust
            "lmx status",
```

with:

```rust
            "lmx status",
            "lmx doctor",
```

**Step 9: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx`

Expected: 87 tests pass, among them `doctor_without_lmxd_reports_the_owner_and_reads_the_markers` and
`doctor_and_the_short_status_follow_the_conditions_of_lmxd`.

---

## Task 7: `lmx net check` and `lmx logs`

**Files:**
- Create: `crates/lmx/src/net.rs`, `crates/lmx/src/logs.rs`
- Modify: `crates/lmx/src/system.rs`, `crates/lmx/src/cli.rs`, `crates/lmx/src/main.rs`, `crates/lmx/src/help.rs`
- Test: `crates/lmx/tests/cli.rs`

`net check` follows the guide "Check a connection in order" inside the guest: the declared firewall rule and its
protocol, the listener and its address, the process. `logs` prints the latest run of a task kind from the journal.

**Step 1: Write the failing tests**

In `crates/lmx/tests/cli.rs`, a fake `journalctl` joins the shared fakes. It answers only the query of `lmx logs apply`
with the records of two builds, and fails like journalctl without permission for any other. The tests at the end check a
listener that only the guest reaches, with its process and user from the prepared `/proc` and `/etc/passwd`, and the
latest of two builds:

Replace

```rust
/// Paths of the fake `ip` and `systemctl`.
struct Tools {
```

with:

```rust
/// Paths of the fake `ip`, `systemctl` and `journalctl`.
struct Tools {
```

and

```rust
    systemctl: PathBuf,
}
```

with:

```rust
    systemctl: PathBuf,
    /// Prints the records of two builds for `lmx logs apply`, and fails like journalctl without
    /// permission for anything else.
    journalctl: PathBuf,
}
```

and

```rust
            ),
        }
```

with:

```rust
            ),
            journalctl: fake_script(
                &directory,
                "journalctl",
                &format!(
                    "#!/bin/sh\n\
                     case \"$*\" in\n\
                     \x20 '') ;;\n\
                     \x20 *'-b0 _UID=0 LMX_KIND=SystemApply') printf '%s\\n' '{JOURNAL}' ;;\n\
                     \x20 *) echo 'No journal files were opened due to insufficient permissions.' >&2; exit 1 ;;\n\
                     esac\n"
                ),
            ),
        }
```

and

```rust
/// Installs an executable that prints `output` into `directory`, runs it once and returns its path.
///
```

with:

```rust
/// Installs an executable that prints `output` into `directory`, runs it once and returns its path.
fn fake_tool(directory: &Path, name: &str, output: &str) -> PathBuf {
    assert!(!output.contains('\''), "tool output is single-quoted");
    fake_script(
        directory,
        name,
        &format!("#!/bin/sh\nprintf '%s\\n' '{output}'\n"),
    )
}

/// Installs `script` as the executable `name` in `directory`, runs it once without arguments and
/// returns its path.
///
```

and

```rust
/// tool is replaced whole and a current one is left alone.
fn fake_tool(directory: &Path, name: &str, output: &str) -> PathBuf {
    assert!(!output.contains('\''), "tool output is single-quoted");
    let path = directory.join(name);
```

with:

```rust
/// tool is replaced whole and a current one is left alone.
fn fake_script(directory: &Path, name: &str, script: &str) -> PathBuf {
    let path = directory.join(name);
```

and

```rust
    let path = directory.join(name);
    let script = format!("#!/bin/sh\nprintf '%s\\n' '{output}'\n");
    let current = fs::read_to_string(&path).ok().as_deref() == Some(script.as_str())
        && fs::metadata(&path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0);
```

with:

```rust
    let path = directory.join(name);
    let current = fs::read_to_string(&path).ok().as_deref() == Some(script)
        && fs::metadata(&path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0);
```

and

```rust
        fs::create_dir_all(directory).expect("create the tool directory");
        fs::write(&staged, &script).expect("write the tool");
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))
```

with:

```rust
        fs::create_dir_all(directory).expect("create the tool directory");
        fs::write(&staged, script).expect("write the tool");
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))
```

and

```rust
    path
}

/// Mount table with the root disk and two shared folders.
```

with:

```rust
    path
}

/// Journal records of two builds of one `lmxd`, as `journalctl -o json` prints them.
const JOURNAL: &str = concat!(
    r#"{"MESSAGE":"applying generation 0123456789aa","LMX_TASK":"system-apply-1","LMX_GENERATION":"0123456789aa","_PID":"812","__REALTIME_TIMESTAMP":"1791364000000000"}"#,
    "\n",
    r#"{"MESSAGE":"applying generation 0123456789ab","LMX_TASK":"system-apply-2","LMX_GENERATION":"0123456789ab","_PID":"812","__REALTIME_TIMESTAMP":"1791364323000000"}"#,
    "\n",
    r#"{"MESSAGE":"building the system configuration...","LMX_TASK":"system-apply-2","_PID":"812","__REALTIME_TIMESTAMP":"1791364324000000"}"#,
);

/// Socket table with a TCP listener on `127.0.0.1:8080` of user 1000, socket inode 4242.
const TCP: &str = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4242 1 0 100 0 0 10 0
";

/// Mount table with the root disk and two shared folders.
```

and

```rust
                "systemd_run": "/run/current-system/sw/bin/systemd-run",
                "journalctl": "/run/current-system/sw/bin/journalctl"
            }
```

with:

```rust
                "systemd_run": "/run/current-system/sw/bin/systemd-run",
                "journalctl": tools.journalctl
            }
```

and

```rust
    assert_eq!(String::from_utf8_lossy(&short.stdout), "lmxd?\n");
}
```

with:

```rust
    assert_eq!(String::from_utf8_lossy(&short.stdout), "lmxd?\n");
}

#[test]
fn net_check_finds_a_listener_that_only_the_guest_reaches() {
    let guest = Guest::new();
    guest.write("proc/net/tcp", TCP);
    guest.write("proc/4242/comm", "python3\n");
    fs::create_dir_all(guest.path("proc/4242/fd")).expect("create fd");
    symlink("socket:[4242]", guest.path("proc/4242/fd/3")).expect("link the socket");
    guest.write(
        "etc/passwd",
        "root:x:0:0::/root:/bin/sh\ndev:x:1000:100::/home/dev:/bin/sh\n",
    );

    let output = guest.lmx(&["net", "check", "8080", "--json"], &guest.config());
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let answer: Value = serde_json::from_slice(&output.stdout).expect("one JSON answer");
    assert_eq!(
        answer["data"],
        json!({
            "port": 8080,
            "protocol": "tcp",
            "checks": [
                {
                    "check": "firewall",
                    "status": "ok",
                    "message": "TCP 8080 is open in the guest firewall."
                },
                {
                    "check": "listener",
                    "status": "failed",
                    "message": "TCP 8080 listens on 127.0.0.1 only, so it is reachable only inside the guest.",
                    "hint": "Make the application listen on 0.0.0.0 or the guest address."
                },
                {
                    "check": "process",
                    "status": "ok",
                    "message": "python3 (pid 4242) holds the socket of user dev."
                }
            ]
        })
    );
}

#[test]
fn logs_show_the_latest_run_of_a_kind() {
    let guest = Guest::new();
    let output = guest.lmx(&["logs", "apply"], &guest.config());
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "system-apply-2, generation 0123456789ab, 2026-10-07 09:12:03 UTC\n\
         applying generation 0123456789ab\n\
         building the system configuration...\n"
    );

    // The fake fails for any other query, as journalctl does for a user without access.
    let output = guest.lmx(&["logs", "health"], &guest.config());
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("run sudo lmx logs health"),
        "{output:?}"
    );
}
```

**Step 2: Run them to see them fail**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx --test cli --no-fail-fast`

Expected: the new tests fail with ``error: unrecognized subcommand 'net'`` and ``'logs'``.

**Step 3: Find the system's paths**

In `crates/lmx/src/system.rs`, the imports:

```rust
use lmx_facts::{
    disk::STORE_PATH, generations::GenerationPaths, machine::MEMINFO_PATH, mounts::MOUNTINFO_PATH,
};
```

with:

```rust
use lmx_facts::{
    disk::STORE_PATH, generations::GenerationPaths, machine::MEMINFO_PATH, mounts::MOUNTINFO_PATH,
    sockets::PROC_PATH,
};
```

the process file system and the user database:

```rust
    /// The process file system.
    pub(crate) fn proc(&self) -> PathBuf {
        self.root.join(PROC_PATH.trim_start_matches('/'))
    }

    /// The user database, for user names.
    pub(crate) fn passwd(&self) -> PathBuf {
        self.root.join("etc/passwd")
    }
```

and `journalctl`, from `PATH` without a configuration, as `ip` and `systemctl`:

```rust
    /// `journalctl` from the configuration, or from `PATH` when the configuration is unreadable.
    pub(crate) fn journalctl(&self) -> PathBuf {
        self.config.as_ref().map_or_else(
            |_| PathBuf::from("journalctl"),
            |config| PathBuf::from(&config.tools.journalctl),
        )
    }
```

**Step 4: Check a port**

Create `crates/lmx/src/net.rs`. A process of another user is invisible without root, so its check is `unknown` with a
hint; Docker's proxy is a warning, because `network.ports` does not control what Docker publishes:

```rust
//! `lmx net check PORT`: why a port of the VM may be unreachable from the Mac.
//!
//! It automates the guest side of the guide "Check a connection in order": the firewall rule and
//! its protocol, the listener and its address, and the process behind it. The VM's address and a
//! connection attempt from the Mac belong to `limanix net check` on the host.

use std::{collections::BTreeSet, fs, io, net::IpAddr, process::ExitCode};

use lmx_facts::{
    FactError,
    sockets::{self, Holder, Listener},
};
use lmx_model::{Check, CheckStatus, Envelope, NetCheck, Ports, Protocol};

use crate::{
    findings, output,
    palette::{Paint, Palette},
    system::System,
};

/// Command name of Docker's proxy, which holds the sockets of ports Docker publishes.
const DOCKER_PROXY: &str = "docker-proxy";

/// Runs `lmx net check PORT`.
pub(crate) fn check(
    system: &System,
    port: u16,
    protocol: Protocol,
    json: bool,
) -> io::Result<ExitCode> {
    let checks = checks(system, port, protocol);
    if json {
        output::write_json(&Envelope::success(NetCheck {
            port,
            protocol,
            checks: checks.clone(),
        }))?;
        return Ok(findings::status(&checks));
    }
    findings::write(&checks, Paint::detect(Palette::of_config(&system.config)))
}

/// The checks of `port`, in the order of the guide.
fn checks(system: &System, port: u16, protocol: Protocol) -> Vec<Check> {
    let ports = system
        .config
        .as_ref()
        .map(|config| &config.network.ports)
        .map_err(Clone::clone);
    let listeners = sockets::listeners(&system.proc(), protocol, port);
    let held = match &listeners {
        Ok(listeners) if !listeners.is_empty() => Some(holders(system, listeners)),
        _ => None,
    };
    let docker = held.as_ref().is_some_and(|(holders, _)| {
        holders
            .iter()
            .any(|(holder, _)| holder.command == DOCKER_PROXY)
    });
    let mut checks = vec![
        firewall(ports, port, protocol, docker),
        listener(&listeners, port, protocol),
    ];
    if let Some((holders, complete)) = held {
        checks.push(process(&holders, complete, port, protocol, |uid| {
            user_name(system, uid)
        }));
    }
    checks
}

/// Whether the guest firewall opens `port` for `protocol`, from the declared ports; a port that
/// Docker publishes passes Docker's own rules instead.
fn firewall(ports: Result<&Ports, String>, port: u16, protocol: Protocol, docker: bool) -> Check {
    let name = protocol.name();
    if docker {
        return Check::new(
            "firewall",
            CheckStatus::Ok,
            format!(
                "Docker publishes {name} {port} with its own rules, outside the guest firewall."
            ),
        );
    }
    let ports = match ports {
        Ok(ports) => ports,
        Err(error) => {
            return Check::new(
                "firewall",
                CheckStatus::Failed,
                format!("The declared ports cannot be read: {error}"),
            )
            .hint("Run lmx doctor.");
        }
    };
    let (open, other, other_name) = match protocol {
        Protocol::Tcp => (&ports.tcp, &ports.udp, Protocol::Udp.name()),
        Protocol::Udp => (&ports.udp, &ports.tcp, Protocol::Tcp.name()),
    };
    let declare = format!(
        "Add {port} to network.ports.{} in limanix.toml, then run limanix update.",
        name.to_lowercase()
    );
    if protocol == Protocol::Tcp && port == 22 {
        Check::new(
            "firewall",
            CheckStatus::Ok,
            "TCP 22 is always open for SSH.",
        )
    } else if open.contains(&port) {
        Check::new(
            "firewall",
            CheckStatus::Ok,
            format!("{name} {port} is open in the guest firewall."),
        )
    } else if other.contains(&port) {
        Check::new(
            "firewall",
            CheckStatus::Failed,
            format!("Only {other_name} {port} is open in the guest firewall, not {name}."),
        )
        .hint(declare)
    } else {
        Check::new(
            "firewall",
            CheckStatus::Failed,
            format!("{name} {port} is not open in the guest firewall."),
        )
        .hint(declare)
    }
}

/// Whether something listens on `port`, and on which addresses.
fn listener(listeners: &Result<Vec<Listener>, FactError>, port: u16, protocol: Protocol) -> Check {
    let name = protocol.name();
    let listeners = match listeners {
        Ok(listeners) => listeners,
        Err(error) => {
            return Check::new("listener", CheckStatus::Failed, error.to_string());
        }
    };
    if listeners.is_empty() {
        return Check::new(
            "listener",
            CheckStatus::Failed,
            format!("Nothing listens on {name} {port} in the guest."),
        )
        .hint("Start the application, and check the port it uses.");
    }
    let addresses: BTreeSet<IpAddr> = listeners
        .iter()
        .map(|listener| listener.address.to_canonical())
        .collect();
    let shown = addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" and ");
    if addresses.iter().all(IpAddr::is_loopback) {
        Check::new(
            "listener",
            CheckStatus::Failed,
            format!(
                "{name} {port} listens on {shown} only, so it is reachable only inside the guest."
            ),
        )
        .hint("Make the application listen on 0.0.0.0 or the guest address.")
    } else {
        Check::new(
            "listener",
            CheckStatus::Ok,
            format!("{name} {port} listens on {shown}."),
        )
    }
}

/// The processes that hold any of `listeners`, each with the user that created its socket, and
/// whether every process could be searched.
fn holders(system: &System, listeners: &[Listener]) -> (Vec<(Holder, u32)>, bool) {
    let inodes: Vec<u64> = listeners.iter().map(|listener| listener.inode).collect();
    let Ok((holders, complete)) = sockets::holders(&system.proc(), &inodes) else {
        return (Vec::new(), false);
    };
    let mut found: Vec<(Holder, u32)> = holders
        .into_iter()
        .map(|holder| {
            let uid = listeners
                .iter()
                .find(|listener| listener.inode == holder.inode)
                .map_or(0, |listener| listener.uid);
            (holder, uid)
        })
        .collect();
    found.dedup_by_key(|(holder, _)| holder.pid);
    (found, complete)
}

/// Which process holds the socket; Docker's proxy means Docker publishes the port.
fn process(
    holders: &[(Holder, u32)],
    complete: bool,
    port: u16,
    protocol: Protocol,
    user_name: impl Fn(u32) -> String,
) -> Check {
    let Some((first, uid)) = holders.first() else {
        if complete {
            return Check::new(
                "process",
                CheckStatus::Ok,
                "No process holds the socket; the kernel does.",
            );
        }
        let udp = if protocol == Protocol::Udp {
            " --udp"
        } else {
            ""
        };
        return Check::new(
            "process",
            CheckStatus::Unknown,
            "No visible process holds the socket.",
        )
        .hint(format!(
            "Run sudo lmx net check {port}{udp} to see processes of other users."
        ));
    };
    let more = match holders.len() {
        1 => String::new(),
        count => format!(" and {} more", count - 1),
    };
    let owner = format!("{} (pid {}){more}", first.command, first.pid);
    let socket = format!("the socket of user {}", user_name(*uid));
    if holders
        .iter()
        .any(|(holder, _)| holder.command == DOCKER_PROXY)
    {
        Check::new(
            "process",
            CheckStatus::Warning,
            format!("Docker publishes {} {port}: {owner}.", protocol.name()),
        )
        .hint(
            "network.ports does not control ports that Docker publishes, and removing the port \
             from it does not close them.",
        )
    } else {
        Check::new(
            "process",
            CheckStatus::Ok,
            format!("{owner} holds {socket}."),
        )
    }
}

/// Name of the user `uid` from the guest's `/etc/passwd`, or `uid N`.
fn user_name(system: &System, uid: u32) -> String {
    fs::read_to_string(system.passwd())
        .ok()
        .and_then(|passwd| {
            passwd.lines().find_map(|line| {
                let mut fields = line.split(':');
                let name = fields.next()?;
                (fields.nth(1)?.parse::<u32>().ok()? == uid).then(|| name.to_owned())
            })
        })
        .unwrap_or_else(|| format!("uid {uid}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Declared ports.
    fn ports(tcp: &[u16], udp: &[u16]) -> Ports {
        Ports {
            tcp: tcp.to_vec(),
            udp: udp.to_vec(),
        }
    }

    /// A listener on `address`.
    fn on(address: [u8; 4]) -> Listener {
        Listener {
            address: IpAddr::from(address),
            port: 8080,
            uid: 1000,
            inode: 1,
        }
    }

    #[test]
    fn the_firewall_opens_a_declared_port_for_its_protocol_only() {
        let declared = ports(&[8080], &[5353]);
        let status = |port, protocol| firewall(Ok(&declared), port, protocol, false).status;
        assert_eq!(status(8080, Protocol::Tcp), CheckStatus::Ok);
        assert_eq!(status(22, Protocol::Tcp), CheckStatus::Ok);
        assert_eq!(status(5353, Protocol::Tcp), CheckStatus::Failed);
        assert_eq!(
            firewall(Ok(&declared), 5353, Protocol::Tcp, false).message,
            "Only UDP 5353 is open in the guest firewall, not TCP."
        );
        assert_eq!(status(9000, Protocol::Udp), CheckStatus::Failed);
        let docker = firewall(Ok(&declared), 9000, Protocol::Tcp, true);
        assert_eq!(docker.status, CheckStatus::Ok, "Docker's rules open it");
    }

    #[test]
    fn a_loopback_listener_is_reachable_only_inside_the_guest() {
        let status = |listeners: Vec<Listener>| listener(&Ok(listeners), 8080, Protocol::Tcp);
        assert_eq!(status(vec![]).status, CheckStatus::Failed);
        let loopback = status(vec![on([127, 0, 0, 1])]);
        assert_eq!(loopback.status, CheckStatus::Failed);
        assert_eq!(
            loopback.message,
            "TCP 8080 listens on 127.0.0.1 only, so it is reachable only inside the guest."
        );
        assert_eq!(
            status(vec![on([127, 0, 0, 1]), on([0, 0, 0, 0])]).status,
            CheckStatus::Ok
        );
    }

    #[test]
    fn names_the_process_and_flags_docker() {
        let holder = |pid, command: &str| {
            (
                Holder {
                    pid,
                    command: command.into(),
                    inode: 1,
                },
                1000,
            )
        };
        let name = |_uid| "dev".to_owned();
        let check = process(&[holder(42, "python3")], true, 8080, Protocol::Tcp, name);
        assert_eq!(
            check.message,
            "python3 (pid 42) holds the socket of user dev."
        );
        let check = process(
            &[holder(7, "docker-proxy")],
            true,
            8080,
            Protocol::Tcp,
            name,
        );
        assert_eq!(check.status, CheckStatus::Warning);
        let kernel = process(&[], true, 8080, Protocol::Tcp, name);
        assert_eq!(kernel.status, CheckStatus::Ok, "the kernel holds it");
        let check = process(&[], false, 8080, Protocol::Tcp, name);
        assert_eq!(check.status, CheckStatus::Unknown);
        assert_eq!(
            check.hint.as_deref(),
            Some("Run sudo lmx net check 8080 to see processes of other users.")
        );
    }
}
```

**Step 5: Show the latest run**

Create `crates/lmx/src/logs.rs`:

```rust
//! `lmx logs KIND`: the output of the latest run of an `lmxd` task kind, from the journal.
//!
//! `lmxd` keeps its runs in memory only, and an apply ends with a restart, so the journal is the
//! history: `--previous` reads the boot before the running one, such as the one that built the
//! running generation.

use std::{io, process::ExitCode};

use lmx_facts::journal::{self, Boot, Record};

use crate::{cli::LogKind, output, system::System};

/// Runs `lmx logs KIND`.
pub(crate) fn run(system: &System, kind: LogKind, previous: bool) -> io::Result<ExitCode> {
    let boot = if previous {
        Boot::Previous
    } else {
        Boot::Current
    };
    let name = kind.name();
    let (records, readable) = match journal::records(&system.journalctl(), kind.task_kind(), boot) {
        Ok(answer) => answer,
        Err(error) => {
            eprintln!("lmx: the journal cannot be read: {error}");
            return Ok(ExitCode::from(output::FAILURE));
        }
    };
    match latest(&records) {
        Some(run) => {
            output::write_text(&render(&run))?;
            Ok(ExitCode::SUCCESS)
        }
        None if !readable => {
            eprintln!(
                "lmx: the journal of lmxd is readable by root and the groups wheel and \
                 systemd-journal; run sudo lmx logs {name}"
            );
            Ok(ExitCode::from(output::FAILURE))
        }
        None if previous => {
            eprintln!("lmx: no {name} ran in the previous boot");
            Ok(ExitCode::from(output::FAILURE))
        }
        None => {
            eprintln!("lmx: no {name} ran in this boot; try lmx logs {name} --previous");
            Ok(ExitCode::from(output::FAILURE))
        }
    }
}

/// The records of the latest task in `records`.
///
/// A restarted `lmxd` numbers its tasks from 1 again, so a run is a task name of one process.
fn latest(records: &[Record]) -> Option<Vec<&Record>> {
    let last = records
        .iter()
        .rev()
        .find(|record| !record.task.is_empty())?;
    Some(
        records
            .iter()
            .filter(|record| record.task == last.task && record.pid == last.pid)
            .collect(),
    )
}

/// The run's task, generation and start, then its lines.
fn render(run: &[&Record]) -> String {
    let Some(first) = run.first() else {
        return String::new();
    };
    let generation = run
        .iter()
        .find_map(|record| record.generation.as_deref())
        .map_or_else(String::new, |generation| {
            format!(", generation {generation}")
        });
    let mut text = format!("{}{generation}, {}\n", first.task, utc(first.time));
    for record in run {
        text.push_str(&record.message);
        text.push('\n');
    }
    text
}

/// `micros` since the Unix epoch as a UTC date and time, such as `2026-10-07 09:12:03 UTC`.
fn utc(micros: u64) -> String {
    let seconds = micros / 1_000_000;
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    // Civil date from days since 1970-01-01, after Howard Hinnant's `civil_from_days`.
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let of_era = shifted - era * 146_097;
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record of `task` in process `pid`.
    fn record(pid: u32, task: &str, message: &str, generation: Option<&str>) -> Record {
        Record {
            task: task.into(),
            pid,
            generation: generation.map(Into::into),
            message: message.into(),
            time: 1_791_364_323_000_000,
        }
    }

    #[test]
    fn shows_the_latest_run_of_the_latest_process() {
        let records = [
            record(7, "system-apply-1", "applying generation g1", Some("g1")),
            record(7, "system-apply-2", "old build", None),
            record(9, "system-apply-1", "applying generation g2", Some("g2")),
            record(
                9,
                "system-apply-1",
                "building the system configuration...",
                None,
            ),
            record(9, "", "an event outside a task", None),
        ];
        let run = latest(&records).expect("a run");
        assert_eq!(
            render(&run),
            "system-apply-1, generation g2, 2026-10-07 09:12:03 UTC\n\
             applying generation g2\n\
             building the system configuration...\n"
        );
        assert!(latest(&[]).is_none());
    }

    #[test]
    fn writes_utc_dates() {
        assert_eq!(utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc(951_782_400_000_000), "2000-02-29 00:00:00 UTC");
        assert_eq!(utc(1_791_364_323_000_000), "2026-10-07 09:12:03 UTC");
    }
}
```

**Step 6: Add the commands**

In `crates/lmx/src/cli.rs`, the commands:

```rust
    Doctor(OutputArgs),
```

with:

```rust
    Doctor(OutputArgs),
    /// Check the network inside this VM.
    #[command(subcommand)]
    Net(NetCommand),
    /// Show the output of the latest lmxd task of a kind, from the journal.
    Logs(LogsArgs),
```

their arguments, before `StoreCommand`:

```rust
/// Network checks.
#[derive(Debug, Subcommand)]
pub(crate) enum NetCommand {
    /// Check why a port of this VM may be unreachable from the Mac: firewall, listener and process.
    Check(NetCheckArgs),
}

/// Arguments of `lmx net check`.
#[derive(Debug, Args)]
pub(crate) struct NetCheckArgs {
    /// Port to check.
    #[arg(value_parser = clap::value_parser!(u16).range(1..))]
    pub(crate) port: u16,
    /// Check a UDP port instead of a TCP one.
    #[arg(long)]
    pub(crate) udp: bool,
    /// Output selection.
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

/// Arguments of `lmx logs`.
#[derive(Debug, Args)]
pub(crate) struct LogsArgs {
    /// Kind of lmxd task.
    #[arg(value_enum)]
    pub(crate) kind: LogKind,
    /// Read the boot before the running one, such as the one that built the running generation.
    #[arg(long)]
    pub(crate) previous: bool,
}

/// Kinds of `lmxd` tasks whose output `lmx logs` shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum LogKind {
    /// Builds of a mounted generation.
    Apply,
    /// Health checks of the booted generation.
    Health,
    /// Removals of older generations after a healthy boot.
    Finalize,
    /// Collections of unreferenced store paths.
    Collect,
    /// Reports of the garbage-collector roots.
    Roots,
}

impl LogKind {
    /// Name on the command line, such as `apply`.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Health => "health",
            Self::Finalize => "finalize",
            Self::Collect => "collect",
            Self::Roots => "roots",
        }
    }

    /// Task kind of `lmxd`, as its journal field `LMX_KIND` names it.
    pub(crate) const fn task_kind(self) -> &'static str {
        match self {
            Self::Apply => "SystemApply",
            Self::Health => "SystemHealth",
            Self::Finalize => "SystemFinalize",
            Self::Collect => "StoreCollect",
            Self::Roots => "StoreRoots",
        }
    }
}
```

and their test:

```rust
    #[test]
    fn checks_ports_and_reads_logs_by_kind() {
        let cli = Cli::try_parse_from(["lmx", "net", "check", "8080", "--udp"]).expect("a check");
        let Some(Command::Net(NetCommand::Check(args))) = cli.command else {
            panic!("not a net check");
        };
        assert_eq!((args.port, args.udp), (8080, true));
        assert!(Cli::try_parse_from(["lmx", "net", "check", "0"]).is_err());
        assert!(Cli::try_parse_from(["lmx", "net", "check", "65536"]).is_err());

        let cli = Cli::try_parse_from(["lmx", "logs", "apply", "--previous"]).expect("logs");
        let Some(Command::Logs(args)) = cli.command else {
            panic!("not logs");
        };
        assert_eq!((args.kind, args.previous), (LogKind::Apply, true));
        assert!(Cli::try_parse_from(["lmx", "logs", "build"]).is_err());
    }
```

In `crates/lmx/src/main.rs`, the crate documentation:

```rust
//! | `lmx doctor`          | facts  | findings about the configuration, `lmxd` and generations    |
```

with:

```rust
//! | `lmx doctor`          | facts  | findings about the configuration, `lmxd` and generations    |
//! | `lmx net check PORT`  | facts  | firewall rule, listener and process of a port               |
//! | `lmx logs KIND`       | facts  | the latest run of an `lmxd` task kind, from the journal     |
```

the modules:

```rust
mod layout;
mod output;
```

with:

```rust
mod layout;
mod logs;
mod net;
mod output;
```

the imports:

```rust
use clap::Parser;
```

with:

```rust
use clap::Parser;
use lmx_model::Protocol;
```

and

```rust
    cli::{ApplyArgs, ApplyCommand, Cli, ClipboardCommand, Command, StoreCommand},
```

with:

```rust
    cli::{ApplyArgs, ApplyCommand, Cli, ClipboardCommand, Command, NetCommand, StoreCommand},
```

and the dispatch:

```rust
        Some(Command::Doctor(args)) => doctor::run(&System::from_environment(), &args),
```

with:

```rust
        Some(Command::Doctor(args)) => doctor::run(&System::from_environment(), &args),
        Some(Command::Net(NetCommand::Check(args))) => net::check(
            &System::from_environment(),
            args.port,
            if args.udp {
                Protocol::Udp
            } else {
                Protocol::Tcp
            },
            args.output.json,
        ),
        Some(Command::Logs(args)) => {
            logs::run(&System::from_environment(), args.kind, args.previous)
        }
```

In `crates/lmx/src/help.rs`, the guest page lists `lmx net check PORT`:

```rust
  lmx doctor            Find what is wrong with the guest owner and what to do about it.
```

with:

```rust
  lmx doctor            Find what is wrong with the guest owner and what to do about it.
  lmx net check PORT    Check why a port may be unreachable from the Mac.
```

and its test expects it:

```rust
            "lmx doctor",
```

with:

```rust
            "lmx doctor",
            "lmx net check PORT",
```

**Step 7: Run the tests**

Run: `cargo +1.90.0 test --manifest-path /Users/igoss/Desktop/lima-personal-shared/limanix/lmx/Cargo.toml -p lmx`

Expected: 95 tests pass, among them `net_check_finds_a_listener_that_only_the_guest_reaches` and
`logs_show_the_latest_run_of_a_kind`.

---

## Task 8: Documentation

**Files:**
- Modify: `docs/contract.md`, `README.md`, `ARCHITECTURE.md`, `docs/plans/2026-10-06-guest-owner-design.md`, `docs/plans/2026-10-07-m4-guest-tools-design.md`

The tables below are formatted by `task markdown/fix`; run it after editing, and `task ci/markdown-fmt` to check.

**Step 1: Describe the commands in the host contract**

In `docs/contract.md`: which commands use no error codes, and sections for `lmx doctor` and `lmx net check`. Replace

```markdown
`lmx status` without `--wait` and `lmx version` do not use these codes or the exit statuses `3` and `130`.
Owner operations, `lmx store reserve`, `lmx apply`, `lmx apply cancel` and `lmx status --wait`, use exit status `3` when `lmxd` is unavailable.
```

with:

```markdown
`lmx status` without `--wait`, `lmx version`, `lmx doctor` and `lmx net check` do not use these codes or the exit statuses `3` and `130`.
Owner operations, `lmx store reserve`, `lmx apply`, `lmx apply cancel` and `lmx status --wait`, use exit status `3` when `lmxd` is unavailable.
```

and

```markdown
## `lmx version`
```

with:

```markdown
## `lmx doctor`

`lmx doctor` diagnoses the guest owner and works without `lmxd`.
`data.checks` lists check records in the order the checks ran:

| Field     | Meaning                                              |
| --------- | ---------------------------------------------------- |
| `check`   | Stable name of the check, such as `owner`            |
| `status`  | `ok`, `warning`, `failed`, or `unknown`              |
| `message` | The finding, for people                              |
| `hint`    | What to do next; omitted when there is nothing to do |

`unknown` means the caller lacks the privileges to check, and the hint says to use `sudo`.
The exit status is `1` when any check failed; a warning does not fail the command.

| Check         | Finds                                                                                                  |
| ------------- | ------------------------------------------------------------------------------------------------------ |
| `config`      | Whether `/etc/lmx/config.json` is readable and valid                                                   |
| `owner`       | Whether `lmxd` answers, and whether its version is the version of `lmx`                                |
| `generations` | The generation conditions of `lmxd`, such as `RestartRequired`, or what the markers say without `lmxd` |
| `disk`        | `DiskLow`, when `lmxd` reports it                                                                      |

Example: [doctor](../contract/v1/doctor.json).

## `lmx net check PORT`

`lmx net check PORT [--udp]` checks why a port of the VM may be unreachable from the Mac.
`data` has `port`, `protocol` (`tcp` or `udp`) and `checks`, check records as in `lmx doctor`, with the same exit status.

| Check      | Finds                                                                                                                                                  |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `firewall` | Whether the guest firewall opens the port for its protocol; TCP 22 is always open for SSH, and Docker publishes ports with its own rules               |
| `listener` | Whether something listens on the port, and whether only on a loopback address                                                                          |
| `process`  | Which process holds the socket, and which user created it; Docker's proxy is a warning, because `network.ports` does not control what Docker publishes |

Without a listener there is no `process` check.
The VM's address and a connection from the Mac belong to `limanix net check` on the host.

Example: [net check](../contract/v1/net-check.json).

## `lmx version`
```

**Step 2: Describe them in the README and the contributor map**

In `README.md`, replace

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

with:

```markdown
| Command                 | Answers or does                                                                                                       |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `lmx help`              | The workspace and the commands inside the VM and on the Mac; also `lmx`, `lmx -h`                                     |
| `lmx info`              | The kernel, guest disk, shared folders and failed units                                                               |
| `lmx welcome`           | The summary an interactive shell prints when it starts                                                                |
| `lmx status`            | Desired, built and booted generations; store disk usage; interfaces; failed systemd units; the state of `lmxd`        |
| `lmx doctor`            | Findings about the configuration, `lmxd` and the generations, with what to do next                                    |
| `lmx net check PORT`    | Why a port may be unreachable from the Mac: firewall rule, listener and process                                       |
| `lmx logs KIND`         | The output of the latest `lmxd` task of a kind, such as `apply`, from the journal; `--previous` reads the boot before |
| `lmx version`           | The binary version and the host contract version                                                                      |
| `lmx store reserve`     | Collects unreferenced store paths when space is low; run in `lmxd`, root only                                         |
| `lmx apply -g G`        | Builds the mounted generation G for the next boot; run in `lmxd`, root only; `--follow` streams the build             |
| `lmx apply cancel -g G` | Stops the apply of generation G; root only                                                                            |
| `lmx clipboard copy`    | Copies standard input to the Mac clipboard                                                                            |
| `lmx clipboard paste`   | Prints the Mac clipboard, if the terminal allows reads                                                                |
| `lmx session NAME`      | Opens a named session with the provider that the selected modules configure                                           |
```

and

```markdown
Add `--json` to `status`, `version`, `store reserve`, `apply` and `apply cancel` to answer with the [host contract](docs/contract.md).
`lmx status --wait converged -g G` answers once `lmxd` reports generation G booted, healthy and finalized.
```

with:

```markdown
Add `--json` to `status`, `doctor`, `net check`, `version`, `store reserve`, `apply` and `apply cancel` to answer with the [host contract](docs/contract.md).
`lmx status --short` prints only what needs attention, such as `restart` or `disk-low`, for tmux and the prompt.
`lmx status --wait converged -g G` answers once `lmxd` reports generation G booted, healthy and finalized.
```

and

```markdown
- The clipboard travels through the terminal with OSC 52, or through tmux inside tmux; the terminal on the Mac must allow it.
- Text is colored only on a terminal, never with `NO_COLOR` or `TERM=dumb`.
- Configuration comes only from NixOS (`/etc/lmx/config.json`), never from the host at runtime.
```

with:

```markdown
- The clipboard travels through the terminal with OSC 52, or through tmux inside tmux; the terminal on the Mac must allow it.
- Text is colored only on a terminal, never with `NO_COLOR` or `TERM=dumb`, with the theme of the declaration (Catppuccin Mocha by default).
- Configuration comes only from NixOS (`/etc/lmx/config.json`), never from the host at runtime.
```

In `ARCHITECTURE.md`, replace

```markdown
- One daemon serves the socket at a time: `lmxd` refuses a socket that another daemon still serves, so two daemons never apply at once.
- Only `lmxd` creates its tasks. The Task API on its socket reads them, and root may cancel or delete them.
```

with:

```markdown
- One daemon serves the socket at a time: `lmxd` refuses a socket that another daemon still serves, so two daemons never apply at once.
- `lmxd` writes its journal fields without a prefix: task output carries `LMX_TASK` and `LMX_KIND`, apply events also `LMX_GENERATION`. `lmx logs` depends on these names.
- Colors come from `theme.palette` in the configuration; `lmx` never hard-codes another theme than its Mocha fallback.
- Only `lmxd` creates its tasks. The Task API on its socket reads them, and root may cancel or delete them.
```

and

```markdown
| Contract types      | Configuration, envelope, error codes, status, apply and version                   | [`lmx-model/src/lib.rs`](crates/lmx-model/src/lib.rs)                         |
| Fact readers        | Disk, generations, machine, mounts, network and failed units                      | [`lmx-facts/src/lib.rs`](crates/lmx-facts/src/lib.rs)                         |
| Command line        | Commands, other names, output selection and exit codes                            | [`lmx/src/main.rs`](crates/lmx/src/main.rs)                                   |
```

with:

```markdown
| Contract types      | Configuration, envelope, error codes, status, apply and version                   | [`lmx-model/src/lib.rs`](crates/lmx-model/src/lib.rs)                         |
| Fact readers        | Disk, generations, machine, mounts, network, sockets, journal and failed units    | [`lmx-facts/src/lib.rs`](crates/lmx-facts/src/lib.rs)                         |
| Command line        | Commands, other names, output selection and exit codes                            | [`lmx/src/main.rs`](crates/lmx/src/main.rs)                                   |
```

and

```markdown
| Status              | Collecting facts and rendering them                                               | [`lmx/src/status.rs`](crates/lmx/src/status.rs)                               |
| Guest pages         | Help, info and the welcome for people in the guest                                | [`lmx/src/welcome.rs`](crates/lmx/src/welcome.rs)                             |
```

with:

```markdown
| Status              | Collecting facts and rendering them                                               | [`lmx/src/status.rs`](crates/lmx/src/status.rs)                               |
| Diagnostics         | `lmx doctor` and `lmx net check`, and their check records                         | [`lmx/src/doctor.rs`](crates/lmx/src/doctor.rs)                               |
| Task history        | `lmx logs` from the journal fields of `lmxd`                                      | [`lmx/src/logs.rs`](crates/lmx/src/logs.rs)                                   |
| Guest pages         | Help, info and the welcome for people in the guest                                | [`lmx/src/welcome.rs`](crates/lmx/src/welcome.rs)                             |
```

**Step 3: Record the implementation in the designs**

In `docs/plans/2026-10-06-guest-owner-design.md`, replace

```markdown
M1c. See [the M3 design](2026-10-07-m3-apply-finalize-design.md) and
[the M3 plan](2026-10-07-m3-apply-finalize.md).
```

with:

```markdown
M1c. See [the M3 design](2026-10-07-m3-apply-finalize-design.md) and
[the M3 plan](2026-10-07-m3-apply-finalize.md). M4 is implemented in this repository: `doctor`,
`net check`, `logs`, `status --short`, the theme from the configuration and journal fields without a
prefix; the catalog's theme capability, the tmux and prompt segments and the journald limit follow in
M1c. See [the M4 design](2026-10-07-m4-guest-tools-design.md) and
[the M4 plan](2026-10-07-m4-guest-tools.md).
```

In `docs/plans/2026-10-07-m4-guest-tools-design.md`, replace

```markdown
Status: design agreed on 2026-10-07. It refines M4 of
[the guest owner design](2026-10-06-guest-owner-design.md) (sections 5, 9, 10 and 12) for this
```

with:

```markdown
Status: design agreed on 2026-10-07 and implemented in this repository; see
[the M4 plan](2026-10-07-m4-guest-tools.md). It refines M4 of
[the guest owner design](2026-10-06-guest-owner-design.md) (sections 5, 9, 10 and 12) for this
```

and

```markdown
   - TCP 22: `ok`, SSH is always open.
2. **`listener`.** The sockets on PORT in `/proc/net/{tcp,tcp6}` in state `LISTEN`, or in
```

with:

```markdown
   - TCP 22: `ok`, SSH is always open.
   - A port that `docker-proxy` holds: `ok`, Docker publishes it with its own rules, outside the
     guest firewall.
2. **`listener`.** The sockets on PORT in `/proc/net/{tcp,tcp6}` in state `LISTEN`, or in
```

and

```markdown
   - `0.0.0.0`, `::` or another address: `ok`.
3. **`process`.** The pid, command and user that own the listening socket, found by its inode in
   `/proc/*/fd`.
   - A process of another user is invisible without root: `unknown`.
```

with:

```markdown
   - `0.0.0.0`, `::` or another address: `ok`.
3. **`process`.** The pid and command that hold the listening socket, found by its inode in one
   search of `/proc/*/fd`, and the user that created the socket.
   - A process of another user is invisible without root: `unknown`.
```

and

```markdown
   - A process of another user is invisible without root: `unknown`.
   - `docker-proxy`: `warning`, Docker publishes the port outside `network.ports`, and removing it
```

with:

```markdown
   - A process of another user is invisible without root: `unknown`.
   - No process holds it, and every process was searched: `ok`, the kernel does.
   - `docker-proxy`: `warning`, Docker publishes the port outside `network.ports`, and removing it
```

and

```markdown
- **Reading.** `journalctl -o json --no-pager -b 0` (`-b -1` with `--previous`) with the match
  `LMX_KIND=<Kind>`. The records are grouped by `LMX_TASK`, and the latest run is printed.
- **Output.**
```

with:

```markdown
- **Reading.** `journalctl -o json --no-pager --all -b 0` (`-b -1` with `--previous`) with the
  matches `_UID=0` and `LMX_KIND=<Kind>`: any user may write a record with `LMX_KIND`, but only root's
  count. A run is a task name of one process (`_PID`), because a restarted `lmxd` numbers its tasks
  from 1 again; the latest run is printed.
- **Output.**
```

and

```markdown
- **Failures,** each with exit status 1:
  - no run: no run of the kind in this boot, try `--previous`;
  - an unreadable journal: run `sudo lmx logs KIND`.
- **Contract.** Text only. The host follows an apply live, so `logs` is not part of the host
```

with:

```markdown
- **Failures,** each with exit status 1:
  - no run, also when the journal has no previous boot: no run of the kind in this boot, try
    `--previous`;
  - an unreadable journal, which journalctl reports with a hint or by failing: run
    `sudo lmx logs KIND`.
- **Contract.** Text only. The host follows an apply live, so `logs` is not part of the host
```

and

```markdown
- Output lines of tasks carry `LMX_TASK` and `LMX_KIND`.
- Apply events carry `LMX_TASK`, `LMX_KIND` and `LMX_GENERATION`: the build's start and the outcome.
- Other fields lose the prefix too: `EXIT_CODE`, `ERROR`.
```

with:

```markdown
- Output lines of tasks carry `LMX_TASK` and `LMX_KIND`.
- Apply events carry `LMX_TASK`, `LMX_KIND` and `LMX_GENERATION`: the start, the build's start and
  the outcome. An apply is named when it starts, so one that fails before its build is a run too.
- Health and finalize events carry the same fields: a failed check, also one that failed before its
  task, and the outcome of a finalize.
- Other fields lose the prefix too: `EXIT_CODE`, `ERROR`.
```

and

```markdown
| Where the commands run | In `lmx`, as facts; `lmxd` only changes its journal fields |
| Firewall rule of `net check` | The declared ports in the configuration: one generation renders them and the firewall, so they cannot drift |
| Journal field names | No prefix, through an option in `solti-observe` |
```

with:

```markdown
| Where the commands run | In `lmx`, as facts; `lmxd` only changes its journal fields |
| Firewall rule of `net check` | The ports in the configuration: one generation renders them and the firewall, so they cannot drift. M1c renders the evaluated `networking.firewall` lists, so ports that modules open count; Docker's ports pass its own rules |
| Journal field names | No prefix, through an option in `solti-observe` |
```

---

## Verification

**Linux.** The CI image cannot see the SDK path, so mount it at the same path. The journald backend of `solti-observe` builds only here. From the host, with `R=/Users/igoss/Desktop/lima-personal-shared/limanix/lmx` and `SD=/Users/igoss/Desktop/projects/solti/sdk`:

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

**Guest.** In the personal LimaNix VM, as transient units with everything under `/tmp/lmx-m4`, and a configuration that names the real tools and declares TCP 8080:

1. **Doctor.** As root, `lmx doctor` reports `config` and `owner` ok with a system daemon running, and as the dev user the generations are `unknown` without `lmxd`.
1. **Net check.** `lmx net check 22` finds `sshd` on `0.0.0.0` as root and `unknown` as the dev user; a port with a loopback listener fails as reachable only inside the guest.
1. **Short status.** It prints nothing or a word, never waits more than 200 ms, and prints `lmxd?` without a daemon.
1. **Logs.** After a transient daemon ran a task, `journalctl -o json LMX_KIND=…` shows `LMX_TASK` without a prefix, and `sudo lmx logs …` prints the run.

Remove the units and `/tmp/lmx-m4` afterwards.

## Suggested commits

The user commits. In the SDK, one commit: `feat(observe): journald field prefix option`. In this repository, one commit per task from Task 2 keeps every commit building and green:
1. `feat(model): declared ports, theme and journalctl; check records`
1. `feat(facts): listening sockets and journal records`
1. `feat(lmxd): journal fields LMX_TASK, LMX_KIND and LMX_GENERATION`
1. `feat(lmx): colors from the declared theme`
1. `feat(lmx): doctor and status --short`
1. `feat(lmx): net check and logs`
1. `docs: M4 design, plan, contract and contributor map`
