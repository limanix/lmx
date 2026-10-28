//! Guest locations that `lmxd` reads and writes, below a system root.

use std::path::PathBuf;

use lmx_facts::{
    generations::{GenerationPaths, PROFILES_PATH},
    mounts::MOUNTINFO_PATH,
};
use lmx_model::Generations;

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

    /// Inputs of one validated generation, independent of the mutable `current` selection.
    pub(crate) fn generation(&self, generation: &str) -> Option<PathBuf> {
        lmx_facts::generations::valid_identifier(generation)
            .then(|| self.at("/mnt/limanix").join(generation))
    }

    /// Inputs of `generation` with the generation markers, `desired` read from those inputs. A name
    /// that is not an identifier, or inputs of another generation, give the marker found instead.
    pub(crate) fn candidate(
        &self,
        generation: &str,
    ) -> Result<(PathBuf, Generations), Option<String>> {
        let inputs = self.generation(generation).ok_or(None)?;
        let mut paths = self.generations();
        paths.desired = inputs.join("flake/runtime.json");
        let (generations, _) = lmx_facts::generations::read(&paths);
        if generations.desired.as_deref() == Some(generation) {
            Ok((inputs, generations))
        } else {
            Err(generations.desired)
        }
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
