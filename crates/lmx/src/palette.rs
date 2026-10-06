//! Colors of the guest's text: Catppuccin Mocha, the palette of the platform prompt.

use std::{
    env,
    io::{self, IsTerminal},
};

/// One palette color as 24-bit RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Color(u8, u8, u8);

/// Mocha `blue`: commands and the logo.
pub(crate) const BLUE: Color = Color(137, 180, 250);
/// Mocha `mauve`: the second half of the logo.
pub(crate) const MAUVE: Color = Color(203, 166, 247);
/// Mocha `subtext0`: secondary values.
pub(crate) const SUBTEXT: Color = Color(166, 173, 200);
/// Mocha `overlay1`: labels.
pub(crate) const MUTED: Color = Color(127, 132, 156);
/// Mocha `green`: writable shared folders.
pub(crate) const GREEN: Color = Color(166, 227, 161);
/// Mocha `peach`: read-only shared folders.
pub(crate) const PEACH: Color = Color(250, 179, 135);
/// Mocha `yellow`: warnings.
pub(crate) const YELLOW: Color = Color(249, 226, 175);

/// Whether text is colored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Paint {
    /// `true` when escape sequences are written.
    enabled: bool,
}

impl Paint {
    /// Colors only a terminal that is not `dumb`, and never when `NO_COLOR` is set; an empty `TERM`
    /// counts as `dumb`, as in the platform prompt.
    pub(crate) fn detect() -> Self {
        let terminal = io::stdout().is_terminal();
        let no_color = env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
        let dumb = env::var_os("TERM").is_none_or(|term| term.is_empty() || term == "dumb");
        Self {
            enabled: terminal && !no_color && !dumb,
        }
    }

    /// Plain text, for tests.
    #[cfg(test)]
    pub(crate) const fn plain() -> Self {
        Self { enabled: false }
    }

    /// Colored text, for tests.
    #[cfg(test)]
    pub(crate) const fn colored() -> Self {
        Self { enabled: true }
    }

    /// `text` in `color`.
    pub(crate) fn color(self, color: Color, text: &str) -> String {
        if self.enabled && !text.is_empty() {
            let Color(red, green, blue) = color;
            format!("\x1b[38;2;{red};{green};{blue}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    /// `text` in bold.
    pub(crate) fn bold(self, text: &str) -> String {
        if self.enabled && !text.is_empty() {
            format!("\x1b[1m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_has_no_escapes() {
        assert_eq!(Paint::plain().color(BLUE, "lmx"), "lmx");
        assert_eq!(Paint::plain().bold("dev-box"), "dev-box");
    }

    #[test]
    fn colored_text_uses_true_color() {
        assert_eq!(
            Paint::colored().color(BLUE, "lmx"),
            "\x1b[38;2;137;180;250mlmx\x1b[0m"
        );
        assert_eq!(Paint::colored().bold("x"), "\x1b[1mx\x1b[0m");
        assert_eq!(Paint::colored().color(YELLOW, ""), "");
    }
}
