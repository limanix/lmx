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

    /// Marker locations below `root`, for tests.
    ///
    /// Not for inspecting another machine's tree: the system profile and `/run/booted-system` are
    /// absolute symlinks into `/nix/store`, which resolve on the inspecting machine.
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
