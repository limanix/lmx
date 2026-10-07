//! Guest locations that `lmxd` reads and writes, below a system root.

use std::path::PathBuf;

use lmx_facts::{
    generations::{GenerationPaths, PROFILES_PATH},
    mounts::MOUNTINFO_PATH,
};

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

    /// Mount table of `lmxd`.
    pub(crate) fn mountinfo(&self) -> PathBuf {
        self.at(MOUNTINFO_PATH)
    }
}
