//! Answers of `lmx store` commands.

use serde::{Deserialize, Serialize};

use crate::DiskUsage;

/// Answer of `lmx store reserve`: store disk usage before and after making room.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reserve {
    /// Usage before the reserve.
    pub before: DiskUsage,
    /// Usage after the reserve; equal to `before` when nothing was collected.
    pub after: DiskUsage,
    /// Free bytes gained; zero when usage grew.
    pub freed_bytes: u64,
    /// Whether unreferenced store paths were collected.
    pub collected: bool,
}

/// Details of a `disk.low` failure of `lmx store reserve`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortage {
    /// Usage before the reserve.
    pub before: DiskUsage,
    /// Usage after the reserve.
    pub after: DiskUsage,
    /// Free bytes gained; zero when usage grew.
    pub freed_bytes: u64,
    /// Why the collection failed, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collect_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::{CONTRACT_VERSION, Envelope, ErrorCode, Reserve, Shortage};

    /// The published reserve examples decode and encode without loss.
    #[test]
    fn contract_examples_round_trip() {
        let example = include_str!("../../../contract/v1/store-reserve.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<Reserve> = serde_json::from_str(example).expect("example decodes");
        assert_eq!(envelope.contract, CONTRACT_VERSION);
        assert!(envelope.ok && envelope.data.is_some());
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);

        let example = include_str!("../../../contract/v1/store-reserve-disk-low.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<Reserve> = serde_json::from_str(example).expect("example decodes");
        let error = envelope.error.clone().expect("a failure");
        assert_eq!(error.code, ErrorCode::DiskLow);
        let shortage: Shortage =
            serde_json::from_value(error.details.into()).expect("details are a shortage");
        assert!(shortage.after.below(10));
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);
    }
}
