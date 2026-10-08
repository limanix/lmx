//! Locations and configuration of the running guest.
//!
//! Without a readable configuration, `ip` and `systemctl` are looked up in `PATH`, and
//! `lmx status` still reports interfaces and failed units; its `config` problem marks the answer
//! as degraded.

use std::{
    env,
    path::{Path, PathBuf},
};

use lmx_facts::{
    disk::STORE_PATH, generations::GenerationPaths, machine::MEMINFO_PATH, mounts::MOUNTINFO_PATH,
    sockets::PROC_PATH,
};
use lmx_ipc::SOCKET_PATH;
use lmx_model::{CONFIG_PATH, Config, Help};

/// Guest locations and the platform configuration, resolved once per command.
#[derive(Debug)]
pub(crate) struct System {
    /// Prefix of system paths; `/` outside tests.
    root: PathBuf,
    /// Platform configuration, or why it could not be read.
    pub(crate) config: Result<Config, String>,
    /// Help cards beside the configuration, as NixOS renders them.
    help_path: PathBuf,
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
        let help_path = config_path.with_file_name("help.json");
        Self {
            root,
            config,
            help_path,
        }
    }

    /// Help cards of the selected modules, or why they could not be read.
    pub(crate) fn help(&self) -> Result<Help, String> {
        Help::load(&self.help_path).map_err(|error| error.to_string())
    }

    /// Path whose file system holds the Nix store.
    pub(crate) fn store(&self) -> PathBuf {
        self.root.join(STORE_PATH.trim_start_matches('/'))
    }

    /// Mount table of this process.
    pub(crate) fn mountinfo(&self) -> PathBuf {
        self.root.join(MOUNTINFO_PATH.trim_start_matches('/'))
    }

    /// Memory information.
    pub(crate) fn meminfo(&self) -> PathBuf {
        self.root.join(MEMINFO_PATH.trim_start_matches('/'))
    }

    /// The process file system.
    pub(crate) fn proc(&self) -> PathBuf {
        self.root.join(PROC_PATH.trim_start_matches('/'))
    }

    /// The user database, for user names.
    pub(crate) fn passwd(&self) -> PathBuf {
        self.root.join("etc/passwd")
    }

    /// Socket of the guest owner daemon `lmxd`.
    pub(crate) fn owner_socket(&self) -> PathBuf {
        self.root.join(SOCKET_PATH.trim_start_matches('/'))
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

    /// `journalctl` from the configuration, or from `PATH` when the configuration is unreadable.
    pub(crate) fn journalctl(&self) -> PathBuf {
        self.config.as_ref().map_or_else(
            |_| PathBuf::from("journalctl"),
            |config| PathBuf::from(&config.tools.journalctl),
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
pub(crate) fn hook(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}
