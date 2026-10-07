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
        "tools": {
            "ip": "/run/current-system/sw/bin/ip",
            "systemctl": "/run/current-system/sw/bin/systemctl",
            "nix_store": "/run/current-system/sw/bin/nix-store",
            "nice": "/run/current-system/sw/bin/nice",
            "ionice": "/run/current-system/sw/bin/ionice",
            "grep": "/run/current-system/sw/bin/grep"
        }
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
