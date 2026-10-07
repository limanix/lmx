//! Exit codes and answers on standard output: JSON for the host, text for people.

use std::{
    io::{self, Write},
    process::ExitCode,
};

use lmx_model::{Envelope, ErrorBody};
use serde::Serialize;

/// Exit status of a failed operation; details are in the JSON answer, in the page, or on standard
/// error.
pub(crate) const FAILURE: u8 = 1;

/// Exit status of a usage error.
const USAGE: u8 = 2;

/// Exit status when the guest owner daemon `lmxd` is unavailable.
pub(crate) const UNAVAILABLE: u8 = 3;

/// Writes one envelope as a single JSON line to standard output.
pub(crate) fn write_json<T: Serialize>(envelope: &Envelope<T>) -> io::Result<()> {
    write_json_line(envelope)
}

/// Writes `value` as a single JSON line to standard output, such as one event of a stream.
pub(crate) fn write_json_line<T: Serialize>(value: &T) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}

/// Reports a failed command and exits with `status`: the JSON answer with `error`, or its message
/// on standard error.
pub(crate) fn failure(json: bool, error: ErrorBody, status: u8) -> io::Result<ExitCode> {
    if json {
        write_json(&Envelope::<()>::failure(error))?;
    } else {
        eprintln!("lmx: {}", error.message);
    }
    Ok(ExitCode::from(status))
}

/// Writes text for people to standard output, returning write failures instead of panicking
/// like `print!`.
pub(crate) fn write_text(text: &str) -> io::Result<()> {
    write_bytes(text.as_bytes())
}

/// Writes bytes, such as clipboard contents, to standard output.
pub(crate) fn write_bytes(bytes: &[u8]) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(bytes)?;
    stdout.flush()
}

/// Exit status of a page for people: success when the parts the page needs could be read.
pub(crate) fn page_status(complete: bool) -> ExitCode {
    if complete {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(FAILURE)
    }
}

/// Reports a usage error.
pub(crate) fn usage(usage: &str) -> ExitCode {
    eprintln!("{usage}");
    ExitCode::from(USAGE)
}
