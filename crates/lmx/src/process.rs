//! Replacing `lmx` with another program.

use std::{
    io,
    os::unix::process::CommandExt,
    process::{Command, ExitCode},
};

/// Exit status when the program to run does not exist, as in a shell.
const NOT_FOUND: u8 = 127;
/// Exit status when the program exists but cannot be run, as in a shell.
const NOT_RUNNABLE: u8 = 126;

/// Replaces this process with `command`, which then owns the streams and the exit status.
///
/// Returns only when the program cannot be started.
pub(crate) fn replace(command: &mut Command) -> ExitCode {
    let error = command.exec();
    eprintln!(
        "lmx: cannot run {}: {error}",
        command.get_program().to_string_lossy()
    );
    ExitCode::from(if error.kind() == io::ErrorKind::NotFound {
        NOT_FOUND
    } else {
        NOT_RUNNABLE
    })
}
