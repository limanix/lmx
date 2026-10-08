//! Answer of `lmx version`: a release and the host contract it speaks.

use serde::{Deserialize, Serialize};

/// Release of a binary and the host contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// Release version of the binary, such as `0.1.0`.
    pub version: String,
    /// Host contract version the binary speaks.
    pub contract: u32,
}

#[cfg(test)]
mod tests {
    use crate::{CONTRACT_VERSION, Envelope, Version};

    #[test]
    fn contract_example_round_trips() {
        let example = include_str!("../../../contract/v1/version.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<Version> = serde_json::from_str(example).expect("example decodes");
        assert_eq!(envelope.contract, CONTRACT_VERSION);
        assert!(envelope.ok && envelope.error.is_none());
        assert_eq!(
            envelope.data.as_ref().map(|version| version.contract),
            Some(CONTRACT_VERSION)
        );
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);
    }
}
