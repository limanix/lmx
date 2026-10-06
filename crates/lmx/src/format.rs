//! Value formatting shared by text output.

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
}
