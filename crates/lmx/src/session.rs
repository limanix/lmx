//! `limanix-session` and `lmx session`: open a named session with the selected provider.
//!
//! `limanix shell NAME --session PROJECT` runs `limanix-session PROJECT` in the guest. The name is
//! passed to the provider unchanged, even when it looks like an option, and the provider then owns
//! the terminal and the exit status.

use std::{ffi::OsStr, process::Command, process::ExitCode};

use lmx_model::Config;

use crate::{output, process};

/// Usage of `limanix-session`.
pub(crate) const USAGE: &str = "Usage: limanix-session NAME (one nonempty session name)";

/// Exit status when no provider is configured, as for a missing command in a shell.
const NO_PROVIDER: u8 = 127;

/// Opens session `name` with the configured provider.
pub(crate) fn run(config: &Result<Config, String>, name: &OsStr) -> ExitCode {
    if name.is_empty() {
        return output::usage(USAGE);
    }
    let config = match config {
        Ok(config) => config,
        Err(message) => {
            eprintln!("lmx: {message}");
            return ExitCode::from(output::FAILURE);
        }
    };
    // The platform rendered a missing provider as an empty command.
    let Some(provider) = config
        .session
        .command
        .as_deref()
        .filter(|command| !command.is_empty())
    else {
        eprint!("{}", missing_provider(&config.session.providers));
        return ExitCode::from(NO_PROVIDER);
    };
    process::replace(Command::new(provider).arg(name))
}

/// Explains how to select a provider when none is configured.
fn missing_provider(providers: &[String]) -> String {
    let mut text = String::from("No session provider is configured in this VM.\n");
    if !providers.is_empty() {
        text.push_str(&format!(
            "Select a session provider in nixos.modules: {}\n",
            providers.join(", ")
        ));
        text.push_str(
            "Apply the configuration with limanix update --config <file> before reconnecting.\n",
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_providers_from_the_catalog() {
        let text = missing_provider(&["lmx:tmux".into(), "third-party:sessions".into()]);
        assert!(text.starts_with("No session provider is configured in this VM.\n"));
        assert!(text.contains("nixos.modules: lmx:tmux, third-party:sessions\n"));
        assert!(text.contains("limanix update"));
    }

    #[test]
    fn invents_no_suggestions() {
        assert_eq!(
            missing_provider(&[]),
            "No session provider is configured in this VM.\n"
        );
    }
}
