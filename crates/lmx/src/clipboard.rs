//! `pbcopy`, `pbpaste` and `lmx clipboard`: the Mac clipboard through the terminal.
//!
//! The terminal on the Mac carries the clipboard in OSC 52 escape sequences and must allow them in
//! its settings. Inside tmux, tmux owns the terminal and passes its buffers to the attached client,
//! so both commands hand over to tmux.

use std::{
    env,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    io::Errno,
    termios::{self, LocalModes, OptionalActions, SpecialCodeIndex, Termios},
};

use crate::{output, process, system};

/// Usage of `pbcopy`.
pub(crate) const COPY_USAGE: &str = "Usage: pbcopy < FILE";
/// Usage of `pbpaste`.
pub(crate) const PASTE_USAGE: &str = "Usage: pbpaste";

/// Terminal of the calling session.
const TTY: &str = "/dev/tty";
/// How long `pbpaste` waits for the terminal, which may first ask for permission.
const PASTE_TIMEOUT: Duration = Duration::from_secs(10);
/// Interval between checks for a new tmux buffer.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// OSC 52 request for the clipboard contents.
const PASTE_REQUEST: &[u8] = b"\x1b]52;c;?\x07";
/// Start of an OSC 52 reply.
const REPLY_START: &[u8] = b"\x1b]52;";

/// Copies standard input to the Mac clipboard.
pub(crate) fn copy() -> io::Result<ExitCode> {
    if let Some(tmux) = tmux() {
        return Ok(process::replace(Command::new(tmux).args([
            "load-buffer",
            "-w",
            "-",
        ])));
    }
    let mut data = Vec::new();
    io::stdin().read_to_end(&mut data)?;
    let written = OpenOptions::new()
        .write(true)
        .open(terminal("LMX_TTY_OUT"))
        .and_then(|mut terminal| terminal.write_all(copy_sequence(&data).as_bytes()));
    if written.is_err() {
        eprintln!("pbcopy needs the terminal of a limanix shell session.");
        return Ok(ExitCode::from(output::FAILURE));
    }
    Ok(ExitCode::SUCCESS)
}

/// Prints the Mac clipboard.
pub(crate) fn paste() -> io::Result<ExitCode> {
    let timeout = paste_timeout();
    if let Some(tmux) = tmux() {
        return paste_through_tmux(&tmux, timeout);
    }
    let opened = File::open(terminal("LMX_TTY_IN")).and_then(|reader| {
        let writer = OpenOptions::new()
            .write(true)
            .open(terminal("LMX_TTY_OUT"))?;
        Ok((reader, writer))
    });
    let Ok((reader, mut writer)) = opened else {
        eprintln!("pbpaste needs the terminal of a limanix shell session.");
        return Ok(ExitCode::from(output::FAILURE));
    };
    let reply = {
        // The reply arrives as terminal input without a newline: read it raw and without echo.
        let _raw = RawMode::enter(&reader);
        writer.write_all(PASTE_REQUEST)?;
        writer.flush()?;
        read_reply(&reader, timeout)?
    };
    match decode_reply(&reply) {
        Some(data) => {
            output::write_bytes(&data)?;
            Ok(ExitCode::SUCCESS)
        }
        None => Ok(unanswered()),
    }
}

/// OSC 52 sequence that sets the clipboard to `data`.
fn copy_sequence(data: &[u8]) -> String {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(data))
}

/// Clipboard contents from the first OSC 52 reply in `input`.
///
/// A reply is `ESC ] 52 ; <selection> ; <base64>` ended by BEL or `ESC \`. Keys typed while
/// waiting may surround it. A terminal that refuses reads answers with no data or with `?`.
fn decode_reply(input: &[u8]) -> Option<Vec<u8>> {
    let data = payload(input)?;
    if data.is_empty() || data == b"?" {
        return None;
    }
    STANDARD.decode(data).ok()
}

/// The base64 part of the first complete OSC 52 reply in `input`.
fn payload(input: &[u8]) -> Option<&[u8]> {
    let start = input
        .windows(REPLY_START.len())
        .position(|window| window == REPLY_START)?;
    let reply = &input[start + REPLY_START.len()..];
    // ST is two bytes: a reply ends only with its `\`, which must not be left for the shell.
    let end = (0..reply.len()).find(|&index| match reply[index] {
        b'\x07' => true,
        b'\x1b' => reply.get(index + 1) == Some(&b'\\'),
        _ => false,
    })?;
    let body = &reply[..end];
    Some(&body[body.iter().position(|byte| *byte == b';')? + 1..])
}

/// Reads terminal input until it holds a complete reply, the input ends, or `timeout` passes.
fn read_reply(terminal: &File, timeout: Duration) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut input = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let wait = Timespec::try_from(remaining).map_err(io::Error::other)?;
        let mut ready = [PollFd::new(terminal, PollFlags::IN)];
        match poll(&mut ready, Some(&wait)) {
            Ok(0) => break,
            Ok(_) => {}
            // Interrupted by a signal: wait for the rest of the time.
            Err(Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
        // Without input, the terminal hung up or cannot be polled, as `/dev/tty` on macOS; a read
        // would block past the timeout.
        if !ready[0].revents().contains(PollFlags::IN) {
            break;
        }
        let count = (&*terminal).read(&mut buffer)?;
        if count == 0 {
            break;
        }
        input.extend_from_slice(&buffer[..count]);
        // Base64 holds neither byte that ends a reply, so only a chunk with one needs a new scan;
        // scanning after every chunk would take quadratic time on a large clipboard.
        let chunk = &buffer[..count];
        if chunk.iter().any(|byte| matches!(byte, b'\x07' | b'\\')) && payload(&input).is_some() {
            break;
        }
    }
    Ok(input)
}

/// Asks tmux for the clipboard and prints the buffer it creates.
fn paste_through_tmux(tmux: &Path, timeout: Duration) -> io::Result<ExitCode> {
    let newest = || {
        Command::new(tmux)
            .args(["list-buffers", "-F", "#{buffer_created} #{buffer_name}"])
            .stderr(Stdio::null())
            .output()
            .ok()
            .and_then(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .map(str::to_owned)
            })
    };
    let before = newest();
    if !Command::new(tmux)
        .args(["refresh-client", "-l"])
        .status()?
        .success()
    {
        return Ok(ExitCode::from(output::FAILURE));
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // The new buffer is printed by name, so a buffer created after it is not printed instead.
        if let Some(created) = newest().filter(|newest| Some(newest) != before.as_ref()) {
            let name = created
                .split_once(' ')
                .map_or(created.as_str(), |(_, name)| name);
            return Ok(process::replace(Command::new(tmux).args([
                "save-buffer",
                "-b",
                name,
                "-",
            ])));
        }
        thread::sleep(POLL_INTERVAL);
    }
    Ok(unanswered())
}

/// Explains that the terminal did not answer.
fn unanswered() -> ExitCode {
    eprintln!(
        "The terminal did not share its clipboard. Allow clipboard reads in its settings, or paste with Cmd+V."
    );
    ExitCode::from(output::FAILURE)
}

/// tmux from `PATH`, when the caller runs inside a tmux session.
fn tmux() -> Option<PathBuf> {
    env::var_os("TMUX").filter(|value| !value.is_empty())?;
    env::split_paths(&env::var_os("PATH")?)
        .map(|directory| directory.join("tmux"))
        .find(|candidate| {
            candidate.metadata().is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}

/// The terminal device, or the file the test hook `name` points to instead.
fn terminal(name: &str) -> PathBuf {
    system::hook(name).unwrap_or_else(|| PathBuf::from(TTY))
}

/// The paste timeout, or the whole seconds of the `LMX_PASTE_TIMEOUT` test hook.
fn paste_timeout() -> Duration {
    env::var("LMX_PASTE_TIMEOUT")
        .ok()
        .and_then(|seconds| seconds.parse::<u32>().ok())
        .map_or(PASTE_TIMEOUT, |seconds| Duration::from_secs(seconds.into()))
}

/// Terminal mode without echo and line buffering, restored when dropped.
///
/// A file that is not a terminal, such as one a test hook points to, is left alone.
#[derive(Debug)]
struct RawMode<'a> {
    /// Terminal whose mode changed.
    terminal: &'a File,
    /// Mode to restore, when the file is a terminal.
    saved: Option<Termios>,
}

impl<'a> RawMode<'a> {
    /// Turns off echo and line buffering on `terminal`.
    fn enter(terminal: &'a File) -> Self {
        let saved = termios::tcgetattr(terminal).ok();
        if let Some(mode) = &saved {
            let mut raw = mode.clone();
            raw.local_modes
                .remove(LocalModes::ECHO | LocalModes::ICANON);
            // As `cfmakeraw`: a read returns as soon as one byte arrives.
            raw.special_codes[SpecialCodeIndex::VMIN] = 1;
            raw.special_codes[SpecialCodeIndex::VTIME] = 0;
            // A terminal that refuses the change still answers, only echoed.
            let _ = termios::tcsetattr(terminal, OptionalActions::Now, &raw);
        }
        Self { terminal, saved }
    }
}

impl Drop for RawMode<'_> {
    fn drop(&mut self) {
        if let Some(mode) = &self.saved {
            let _ = termios::tcsetattr(self.terminal, OptionalActions::Now, mode);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_clipboard_for_the_terminal() {
        assert_eq!(copy_sequence(b"hello\n"), "\x1b]52;c;aGVsbG8K\x07");
    }

    #[test]
    fn decodes_replies_ended_by_bel_or_st() {
        assert_eq!(
            decode_reply(b"\x1b]52;c;aGVsbG8gd29ybGQ=\x07").as_deref(),
            Some(&b"hello world"[..])
        );
        assert_eq!(
            decode_reply(b"\x1b]52;c;aGVsbG8=\x1b\\").as_deref(),
            Some(&b"hello"[..])
        );
    }

    #[test]
    fn waits_for_the_whole_string_terminator() {
        assert_eq!(payload(b"\x1b]52;c;aGVsbG8=\x1b"), None);
        assert_eq!(payload(b"\x1b]52;c;aGVsbG8=\x1b\\"), Some(&b"aGVsbG8="[..]));
    }

    #[test]
    fn ignores_keys_typed_around_the_reply() {
        assert_eq!(
            decode_reply(b"ls\x1b]52;c;aGVsbG8=\x07\r").as_deref(),
            Some(&b"hello"[..])
        );
    }

    #[test]
    fn treats_refusals_and_other_input_as_unanswered() {
        for input in [
            &b""[..],
            b"\x1b]52;c;?\x07",
            b"\x1b]52;c;\x07",
            b"\x1b]52;c;aGVsbG8=",
            b"\x1b]11;rgb:0000/0000/0000\x07",
            b"\x1b]52;c;not base64!\x07",
        ] {
            assert_eq!(decode_reply(input), None, "{input:?}");
        }
    }
}
