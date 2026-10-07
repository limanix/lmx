//! Check records of `lmx doctor` and `lmx net check`, for people and for the host.

use std::{io, process::ExitCode};

use lmx_model::{Check, CheckStatus};

use crate::{output, palette::Paint};

/// Width of the status column.
const STATUS: usize = 8;

/// Width of the check column.
const CHECK: usize = 11;

/// Name of a status for people.
const fn name(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Ok => "ok",
        CheckStatus::Warning => "warning",
        CheckStatus::Failed => "failed",
        CheckStatus::Unknown => "unknown",
    }
}

/// Renders `checks` as aligned rows, each hint on its own line under the message.
pub(crate) fn render(checks: &[Check], paint: Paint) -> String {
    let palette = paint.palette();
    let mut text = String::new();
    for check in checks {
        let color = match check.status {
            CheckStatus::Ok => palette.green,
            CheckStatus::Warning => palette.yellow,
            CheckStatus::Failed => palette.red,
            CheckStatus::Unknown => palette.muted,
        };
        let status = format!("{:<STATUS$}", name(check.status));
        text.push_str(&format!(
            "{} {:<CHECK$} {}\n",
            paint.color(color, &status),
            check.check,
            check.message
        ));
        if let Some(hint) = &check.hint {
            text.push_str(&format!(
                "{:indent$}{}\n",
                "",
                paint.color(palette.muted, hint),
                indent = STATUS + CHECK + 2
            ));
        }
    }
    text
}

/// Writes `checks` for people and exits with failure when any check failed.
pub(crate) fn write(checks: &[Check], paint: Paint) -> io::Result<ExitCode> {
    output::write_text(&render(checks, paint))?;
    Ok(status(checks))
}

/// Exit status of an answer with `checks`: a failed check fails the command, a warning does not.
pub(crate) fn status(checks: &[Check]) -> ExitCode {
    output::page_status(
        !checks
            .iter()
            .any(|check| check.status == CheckStatus::Failed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_checks_and_puts_hints_under_the_message() {
        let checks = [
            Check::new("owner", CheckStatus::Ok, "lmxd 0.1.0 answers."),
            Check::new("firewall", CheckStatus::Failed, "TCP 8080 is not open.")
                .hint("Add it to network.ports."),
        ];
        assert_eq!(
            render(&checks, Paint::plain()),
            "ok       owner       lmxd 0.1.0 answers.\n\
             failed   firewall    TCP 8080 is not open.\n\
             \x20                    Add it to network.ports.\n"
        );
    }
}
