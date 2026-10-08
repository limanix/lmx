//! Usage of the file system that holds the Nix store.

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
