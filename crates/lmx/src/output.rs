//! Exit codes and answers on standard output: JSON for the host, text for people.

use std::io::{self, Write};

use lmx_model::Envelope;
use serde::Serialize;

/// Exit status of a failed operation; details are in the JSON answer or on standard error.
pub(crate) const FAILURE: u8 = 1;

/// Writes one envelope as a single JSON line to standard output.
pub(crate) fn write_json<T: Serialize>(envelope: &Envelope<T>) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, envelope)?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}

/// Writes text for people to standard output, returning write failures instead of panicking
/// like `print!`.
pub(crate) fn write_text(text: &str) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(text.as_bytes())?;
    stdout.flush()
}
