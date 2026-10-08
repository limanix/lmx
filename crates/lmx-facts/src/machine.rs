//! The virtual machine itself: processors, memory and kernel.

use std::{fs, path::Path, thread};

use crate::FactError;

/// Memory information inside a booted guest.
pub const MEMINFO_PATH: &str = "/proc/meminfo";

/// Number of processors the caller may use.
pub fn cpus() -> Result<usize, FactError> {
    thread::available_parallelism()
        .map(usize::from)
        .map_err(|source| FactError::Io {
            what: "the processor count",
            source,
        })
}

/// Reads total memory, in bytes, from the memory information at `path`.
pub fn memory(path: &Path) -> Result<u64, FactError> {
    let info = fs::read_to_string(path).map_err(|source| FactError::Io {
        what: "memory information",
        source,
    })?;
    parse_memory(&info)
}

/// Parses `MemTotal` from `/proc/meminfo`, which the kernel reports in kibibytes.
pub fn parse_memory(info: &str) -> Result<u64, FactError> {
    info.lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|value| value.trim().strip_suffix("kB"))
        .and_then(|kibibytes| kibibytes.trim().parse::<u64>().ok())
        .map(|kibibytes| kibibytes.saturating_mul(1024))
        .ok_or_else(|| FactError::Parse {
            what: "memory information",
            detail: "no MemTotal line in kB".into(),
        })
}

/// Kernel name and release.
pub fn kernel() -> String {
    let name = rustix::system::uname();
    format!(
        "{} {}",
        name.sysname().to_string_lossy(),
        name.release().to_string_lossy()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_total_memory_in_bytes() {
        let info = "MemTotal:        7969124 kB\nMemFree:          524288 kB\n";
        assert_eq!(parse_memory(info).expect("MemTotal"), 7_969_124 * 1024);
    }

    #[test]
    fn rejects_memory_information_without_a_total() {
        let error = parse_memory("MemFree: 524288 kB\n").expect_err("no MemTotal");
        assert!(
            error
                .to_string()
                .starts_with("unexpected memory information output"),
            "{error}"
        );
    }

    #[test]
    fn explains_unreadable_memory_information() {
        let error = memory(Path::new("/nonexistent/meminfo")).expect_err("missing file");
        assert!(
            error
                .to_string()
                .starts_with("cannot read memory information: "),
            "{error}"
        );
    }

    #[test]
    fn counts_at_least_one_processor() {
        assert!(cpus().expect("processor count") >= 1);
    }

    #[test]
    fn names_the_running_kernel() {
        let kernel = kernel();
        assert!(kernel.contains(' ') && !kernel.starts_with(' '), "{kernel}");
    }
}
