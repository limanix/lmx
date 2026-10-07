//! Failed systemd units.

use std::path::Path;

use crate::{FactError, command};

/// Lists failed units with `systemctl list-units --state=failed`.
pub fn failed(systemctl: &Path) -> Result<Vec<String>, FactError> {
    let output = command::output(
        systemctl,
        &[
            "list-units",
            "--state=failed",
            "--plain",
            "--no-legend",
            "--no-pager",
        ],
    )?;
    Ok(parse(&String::from_utf8_lossy(&output)))
}

/// Parses plain `systemctl list-units` output: the unit name is the first column of each line.
pub fn parse(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_first_column() {
        let output = "limanix-store-guard.service loaded failed failed Keep free space\n\
                      docker.socket loaded failed failed Docker Socket for the API\n\n";
        assert_eq!(
            parse(output),
            ["limanix-store-guard.service", "docker.socket"]
        );
    }

    #[test]
    fn empty_output_means_no_failures() {
        assert!(parse("").is_empty());
    }
}
