//! Answers of `lmx doctor` and `lmx net check`: one record per check.

use serde::{Deserialize, Serialize};

/// Outcome of one check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    /// The check passed.
    Ok,
    /// Something needs attention, but nothing is broken.
    Warning,
    /// Something is broken; the hint says what to do.
    Failed,
    /// The caller lacks the privileges to check; the hint says to use `sudo`.
    Unknown,
}

/// One check and its finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// Stable name of the check, such as `owner`.
    pub check: String,
    /// Outcome.
    pub status: CheckStatus,
    /// Finding for people.
    pub message: String,
    /// What to do next, when there is something to do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Check {
    /// A check named `check` with `status` and `message`, without a hint.
    #[must_use]
    pub fn new(check: &str, status: CheckStatus, message: impl Into<String>) -> Self {
        Self {
            check: check.to_owned(),
            status,
            message: message.into(),
            hint: None,
        }
    }

    /// The check with `hint`.
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// Answer of `lmx doctor`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Doctor {
    /// The checks in the order they ran.
    pub checks: Vec<Check>,
}

/// Transport protocol of a port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

impl Protocol {
    /// Name for people, such as `TCP`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        }
    }
}

/// Answer of `lmx net check`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetCheck {
    /// The port checked.
    pub port: u16,
    /// Its protocol.
    pub protocol: Protocol,
    /// The checks in the order they ran.
    pub checks: Vec<Check>,
}

#[cfg(test)]
mod tests {
    use crate::{CONTRACT_VERSION, Doctor, Envelope, NetCheck};

    /// The published examples decode and encode without loss.
    #[test]
    fn contract_examples_round_trip() {
        let example = include_str!("../../../contract/v1/doctor.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<Doctor> = serde_json::from_str(example).expect("example decodes");
        assert_eq!(envelope.contract, CONTRACT_VERSION);
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);

        let example = include_str!("../../../contract/v1/net-check.json");
        let original: serde_json::Value = serde_json::from_str(example).expect("example is JSON");
        let envelope: Envelope<NetCheck> = serde_json::from_str(example).expect("example decodes");
        assert_eq!(serde_json::to_value(&envelope).expect("encode"), original);
    }
}
