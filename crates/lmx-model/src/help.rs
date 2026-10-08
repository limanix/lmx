//! Help cards written by NixOS for `lmx help TOPIC`.
//!
//! Catalog modules, and any other module of the declaration, describe themselves with the NixOS
//! option `limanix.help.<topic>`. The LimaNix platform renders the cards of the evaluated system into
//! [`HELP_PATH`], beside the [configuration](crate::Config). The file and the binary come from one
//! generation, so unknown fields are rejected as in the configuration.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// Path of the help cards inside a booted guest, beside [`crate::CONFIG_PATH`].
pub const HELP_PATH: &str = "/etc/lmx/help.json";

/// Help schema understood by this crate.
pub const HELP_SCHEMA: u32 = 1;

/// Help cards of one guest generation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Help {
    /// Schema version; [`Help::from_json`] accepts only [`HELP_SCHEMA`].
    pub schema: u32,
    /// Cards by topic, such as `python`.
    pub topics: BTreeMap<String, Card>,
}

/// What one topic gives the guest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Card {
    /// Heading, such as `Python 3.12.14`.
    pub title: String,
    /// One sentence about the topic.
    pub summary: String,
    /// Commands the topic puts on `PATH`, most used first.
    pub commands: Vec<String>,
    /// Common tasks.
    pub tips: Vec<Tip>,
    /// Address of the full guide, if there is one.
    pub guide: Option<String>,
}

/// One common task of a topic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tip {
    /// Short label, at most 10 characters.
    pub label: String,
    /// A command or a short sentence.
    pub text: String,
}

/// Failure to obtain usable help cards.
#[derive(Debug, thiserror::Error)]
pub enum HelpError {
    /// The file could not be read.
    #[error("cannot read {}: {source}", path.display())]
    Read {
        /// Path that was read.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: io::Error,
    },
    /// The file is not valid help JSON.
    #[error("invalid help cards: {0}")]
    Invalid(#[from] serde_json::Error),
    /// The file uses a schema this binary does not understand.
    #[error("unsupported help schema {found}; this lmx reads schema {HELP_SCHEMA}")]
    Schema {
        /// Schema found in the file.
        found: u32,
    },
}

impl Help {
    /// Reads and validates the help cards at `path`.
    pub fn load(path: &Path) -> Result<Self, HelpError> {
        let data = fs::read(path).map_err(|source| HelpError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_json(&data)
    }

    /// Parses help JSON, checking its schema before its fields.
    pub fn from_json(data: &[u8]) -> Result<Self, HelpError> {
        let Header { schema } = serde_json::from_slice(data)?;
        if schema != HELP_SCHEMA {
            return Err(HelpError::Schema { found: schema });
        }
        Ok(serde_json::from_slice(data)?)
    }

    /// The topic and card that `name` asks for.
    ///
    /// `name` may be a topic such as `python`, a catalog selector such as `lmx:python-3.12`, or a
    /// topic with a version line such as `python-3.12`.
    pub fn find(&self, name: &str) -> Option<(&str, &Card)> {
        let name = name.strip_prefix("lmx:").unwrap_or(name);
        if let Some((topic, card)) = self.topics.get_key_value(name) {
            return Some((topic, card));
        }
        let (topic, line) = name.rsplit_once('-')?;
        if !line.starts_with(|first: char| first.is_ascii_digit()) {
            return None;
        }
        self.topics
            .get_key_value(topic)
            .map(|(topic, card)| (topic.as_str(), card))
    }
}

/// Schema version of a help file, read before its other fields.
#[derive(Deserialize)]
#[serde(expecting = "a help object")]
struct Header {
    /// Schema version of the file.
    schema: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "schema": 1,
        "topics": {
            "python": {
                "title": "Python 3.12.14",
                "summary": "Python 3 with venv, virtualenv, and the Pyright language server.",
                "commands": ["python", "virtualenv", "pyright", "python-3.12"],
                "tips": [{"label": "New venv", "text": "python -m venv .venv"}],
                "guide": "https://limanix.dev/categories/nixos/modules/python/README.html"
            },
            "k9s": {
                "title": "k9s 0.51",
                "summary": "Terminal UI for Kubernetes.",
                "commands": ["k9s"],
                "tips": [],
                "guide": null
            }
        }
    }"#;

    fn sample() -> Help {
        Help::from_json(SAMPLE.as_bytes()).expect("valid help")
    }

    #[test]
    fn parses_the_platform_help() {
        let help = sample();
        assert_eq!(help.topics["python"].title, "Python 3.12.14");
        assert_eq!(help.topics["python"].tips[0].label, "New venv");
        assert_eq!(help.topics["k9s"].guide, None);
    }

    #[test]
    fn finds_a_topic_by_name_selector_or_version_line() {
        let help = sample();
        for name in ["python", "lmx:python", "python-3.12", "lmx:python-3.12"] {
            assert_eq!(
                help.find(name).map(|(topic, _)| topic),
                Some("python"),
                "{name}"
            );
        }
        assert_eq!(help.find("k9s-0.51").map(|(topic, _)| topic), Some("k9s"));
    }

    #[test]
    fn a_suffix_that_is_not_a_version_line_finds_nothing() {
        let help = sample();
        for name in ["python-tools", "pyth", "lmx:", "-3.12", "go"] {
            assert!(help.find(name).is_none(), "{name}");
        }
    }

    #[test]
    fn rejects_another_schema_and_unknown_fields() {
        let other = SAMPLE.replacen("\"schema\": 1", "\"schema\": 2", 1);
        assert!(matches!(
            Help::from_json(other.as_bytes()),
            Err(HelpError::Schema { found: 2 })
        ));
        let extra = SAMPLE.replacen("\"schema\": 1,", "\"schema\": 1, \"extra\": true,", 1);
        assert!(matches!(
            Help::from_json(extra.as_bytes()),
            Err(HelpError::Invalid(_))
        ));
    }
}
