//! Runs one system tool and returns its standard output.

use std::{
    path::Path,
    process::{Command, Output, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use crate::FactError;

/// Longest standard-error tail kept in [`FactError::Command`].
const STDERR_TAIL: usize = 2048;

/// Longest wait for one tool.
///
/// `lmx status` runs `ip` and `systemctl` in turn and the host waits 10 seconds for its answer: a
/// stuck tool, such as `systemctl` while PID 1 does not answer, must still leave time for the other
/// facts.
const TIMEOUT: Duration = Duration::from_secs(3);

/// Runs `program` with `args` and returns standard output if it exits successfully within [`TIMEOUT`].
///
/// Standard input is closed so a tool that unexpectedly prompts fails instead of waiting.
pub(crate) fn output(program: &Path, args: &[&str]) -> Result<Vec<u8>, FactError> {
    output_within(program, args, TIMEOUT)
}

/// Runs `program` like [`output`], waiting at most `timeout`; also returns its standard error, where
/// some tools explain an incomplete answer.
pub(crate) fn output_and_errors(
    program: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<(Vec<u8>, String), FactError> {
    run(program, args, timeout).map(|output| {
        (
            output.stdout,
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    })
}

/// Runs `program` like [`output`], waiting at most `timeout`.
fn output_within(program: &Path, args: &[&str], timeout: Duration) -> Result<Vec<u8>, FactError> {
    run(program, args, timeout).map(|output| output.stdout)
}

/// Runs `program` with `args`, waiting at most `timeout`; a failure is [`FactError::Command`].
///
/// A tool that is still running is left to finish on its own instead of being killed.
fn run(program: &Path, args: &[&str], timeout: Duration) -> Result<Output, FactError> {
    let spawn_error = |source| FactError::Spawn {
        program: program.display().to_string(),
        source,
    };
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .spawn(move || sender.send(command.output()))
        .map_err(spawn_error)?;
    let output = receiver
        .recv_timeout(timeout)
        .map_err(|_| FactError::Timeout {
            program: program.display().to_string(),
            timeout,
        })?
        .map_err(spawn_error)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let start = stderr.len().saturating_sub(STDERR_TAIL);
        let start = (start..stderr.len())
            .find(|index| stderr.is_char_boundary(*index))
            .unwrap_or(stderr.len());
        return Err(FactError::Command {
            program: program.display().to_string(),
            status: output.status,
            stderr: stderr[start..].to_owned(),
        });
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn shell(script: &str, timeout: Duration) -> Result<Vec<u8>, FactError> {
        output_within(Path::new("/bin/sh"), &["-c", script], timeout)
    }

    #[test]
    fn returns_standard_output_of_a_successful_tool() {
        assert_eq!(shell("echo ok", TIMEOUT).expect("sh succeeds"), b"ok\n");
    }

    #[test]
    fn reports_the_exit_status_with_standard_error() {
        let error = shell("echo 'no bus' >&2; exit 3", TIMEOUT).expect_err("sh exits with 3");
        assert_eq!(error.to_string(), "/bin/sh failed (exit status: 3): no bus");
    }

    #[test]
    fn keeps_the_end_of_long_standard_error_on_a_character_boundary() {
        let script = format!("printf %s '{}' >&2; exit 1", "€".repeat(1000));
        let error = shell(&script, TIMEOUT).expect_err("sh exits with 1");
        let FactError::Command { stderr, .. } = error else {
            panic!("not a command failure: {error}");
        };
        assert_eq!(stderr, "€".repeat(682));
    }

    #[test]
    fn reports_a_program_that_cannot_start() {
        let error = output(Path::new("/nonexistent/ip"), &[]).expect_err("no such program");
        assert_eq!(
            error.to_string(),
            "cannot run /nonexistent/ip: No such file or directory (os error 2)"
        );
    }

    #[test]
    fn stops_waiting_for_a_tool_that_hangs() {
        let started = Instant::now();
        let error = shell("exec sleep 2", Duration::from_millis(100)).expect_err("sleep hangs");
        assert!(matches!(error, FactError::Timeout { .. }), "{error}");
        assert!(started.elapsed() < Duration::from_millis(1500));
    }
}
