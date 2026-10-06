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
