//! Value formatting shared by text output.

use lmx_model::DiskUsage;

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

/// Formats the space available to users, as `df` reports it, and, on file systems with an inode
/// table, free inodes.
pub(crate) fn disk(usage: &DiskUsage) -> String {
    let space = format!(
        "{} of {} free",
        gibibytes(usage.available_bytes),
        gibibytes(usage.bytes)
    );
    if usage.inodes == 0 {
        return space;
    }
    format!(
        "{space}, {} of {} inodes free",
        usage.free_inodes, usage.inodes
    )
}

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

    #[test]
    fn leaves_out_inodes_without_an_inode_table() {
        let mut usage = DiskUsage {
            bytes: 16 * GIB,
            free_bytes: 9 * GIB,
            available_bytes: 8 * GIB,
            inodes: 1_048_576,
            free_inodes: 495_616,
        };
        assert_eq!(
            disk(&usage),
            "8.0 GiB of 16 GiB free, 495616 of 1048576 inodes free"
        );
        usage.inodes = 0;
        usage.free_inodes = 0;
        assert_eq!(disk(&usage), "8.0 GiB of 16 GiB free");
    }
}
