//! Colors of the guest's text.

use std::{
    env,
    io::{self, IsTerminal},
};

use lmx_model::{Config, Theme};

/// One color of the guest's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Color {
    /// 24-bit RGB, from a Catppuccin palette.
    Rgb(u8, u8, u8),
    /// One of the terminal's 16 colors, 0 to 15, as the terminal's theme draws it.
    Terminal(u8),
    /// The terminal's default foreground.
    Default,
}

impl Color {
    /// Parses `#rrggbb`.
    fn parse(text: &str) -> Option<Self> {
        let hex = text
            .strip_prefix('#')
            .filter(|hex| hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))?;
        let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
        Some(Self::Rgb(channel(0)?, channel(2)?, channel(4)?))
    }

    /// The SGR parameters that make the color the foreground; none for the default.
    fn foreground(self) -> Option<String> {
        match self {
            Self::Rgb(red, green, blue) => Some(format!("38;2;{red};{green};{blue}")),
            Self::Terminal(index @ 0..=7) => Some((30 + index).to_string()),
            Self::Terminal(index) => Some((90 + index - 8).to_string()),
            Self::Default => None,
        }
    }
}

/// The colors `lmx` uses, by the theme's color names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Palette {
    /// `blue`: commands and the logo.
    pub(crate) blue: Color,
    /// `mauve`: the second half of the logo.
    pub(crate) mauve: Color,
    /// `subtext0`: secondary values.
    pub(crate) subtext: Color,
    /// `overlay1`: labels.
    pub(crate) muted: Color,
    /// `green`: passed checks and writable shared folders.
    pub(crate) green: Color,
    /// `peach`: read-only shared folders.
    pub(crate) peach: Color,
    /// `yellow`: warnings.
    pub(crate) yellow: Color,
    /// `red`: failed checks.
    pub(crate) red: Color,
}

impl Palette {
    /// Catppuccin Mocha, the platform's palette until the declaration chooses a theme.
    pub(crate) const MOCHA: Self = Self {
        blue: Color::Rgb(137, 180, 250),
        mauve: Color::Rgb(203, 166, 247),
        subtext: Color::Rgb(166, 173, 200),
        muted: Color::Rgb(127, 132, 156),
        green: Color::Rgb(166, 227, 161),
        peach: Color::Rgb(250, 179, 135),
        yellow: Color::Rgb(249, 226, 175),
        red: Color::Rgb(243, 139, 168),
    };

    /// The terminal's own colors for the same names, as the catalog's `ansi.toml` assigns them.
    pub(crate) const TERMINAL: Self = Self {
        blue: Color::Terminal(4),
        mauve: Color::Terminal(5),
        subtext: Color::Terminal(8),
        muted: Color::Terminal(8),
        green: Color::Terminal(2),
        peach: Color::Terminal(11),
        yellow: Color::Terminal(3),
        red: Color::Terminal(1),
    };

    /// The palette of `theme`. With the `system` flavor, each color is the terminal color that
    /// `ansi` names, or the terminal's default for `None`; a name it lacks keeps
    /// [`Palette::TERMINAL`]'s. Otherwise a color the palette lacks, or one that is not
    /// `#rrggbb`, stays Mocha's.
    pub(crate) fn of(theme: &Theme) -> Self {
        if theme.flavor == "system" {
            let pick = |name: &str, terminal: Color| match theme.ansi.get(name) {
                Some(Some(index)) if *index < 16 => Color::Terminal(*index),
                Some(None) => Color::Default,
                _ => terminal,
            };
            let terminal = Self::TERMINAL;
            return Self {
                blue: pick("blue", terminal.blue),
                mauve: pick("mauve", terminal.mauve),
                subtext: pick("subtext0", terminal.subtext),
                muted: pick("overlay1", terminal.muted),
                green: pick("green", terminal.green),
                peach: pick("peach", terminal.peach),
                yellow: pick("yellow", terminal.yellow),
                red: pick("red", terminal.red),
            };
        }
        let pick = |name: &str, mocha: Color| {
            theme
                .palette
                .get(name)
                .and_then(|value| Color::parse(value))
                .unwrap_or(mocha)
        };
        let mocha = Self::MOCHA;
        Self {
            blue: pick("blue", mocha.blue),
            mauve: pick("mauve", mocha.mauve),
            subtext: pick("subtext0", mocha.subtext),
            muted: pick("overlay1", mocha.muted),
            green: pick("green", mocha.green),
            peach: pick("peach", mocha.peach),
            yellow: pick("yellow", mocha.yellow),
            red: pick("red", mocha.red),
        }
    }

    /// The palette of the configuration, or Mocha without one.
    pub(crate) fn of_config(config: &Result<Config, String>) -> Self {
        config
            .as_ref()
            .map_or(Self::MOCHA, |config| Self::of(&config.theme))
    }
}

/// Variable that turns colors off when it is set and not empty.
pub(crate) const NO_COLOR: &str = "NO_COLOR";

/// Variable that turns colors off when it is unset, empty or `dumb`.
pub(crate) const TERM: &str = "TERM";

/// Whether and how text is colored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Paint {
    /// `true` when escape sequences are written.
    enabled: bool,
    /// The colors.
    palette: Palette,
}

impl Paint {
    /// Colors with `palette` only a terminal that is not `dumb`, and never when `NO_COLOR` is set;
    /// an empty `TERM` counts as `dumb`, as in the platform prompt.
    pub(crate) fn detect(palette: Palette) -> Self {
        let terminal = io::stdout().is_terminal();
        let no_color = env::var_os(NO_COLOR).is_some_and(|value| !value.is_empty());
        let dumb = env::var_os(TERM).is_none_or(|term| term.is_empty() || term == "dumb");
        Self {
            enabled: terminal && !no_color && !dumb,
            palette,
        }
    }

    /// Plain text, for tests.
    #[cfg(test)]
    pub(crate) const fn plain() -> Self {
        Self {
            enabled: false,
            palette: Palette::MOCHA,
        }
    }

    /// Text colored with Mocha, for tests.
    #[cfg(test)]
    pub(crate) const fn colored() -> Self {
        Self {
            enabled: true,
            palette: Palette::MOCHA,
        }
    }

    /// The colors.
    pub(crate) const fn palette(self) -> Palette {
        self.palette
    }

    /// `text` in `color`; the terminal's default color needs no escape.
    pub(crate) fn color(self, color: Color, text: &str) -> String {
        match color.foreground() {
            Some(foreground) if self.enabled && !text.is_empty() => {
                format!("\x1b[{foreground}m{text}\x1b[0m")
            }
            _ => text.to_owned(),
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
        assert_eq!(Paint::plain().color(Palette::MOCHA.blue, "lmx"), "lmx");
        assert_eq!(Paint::plain().bold("dev-box"), "dev-box");
    }

    #[test]
    fn colored_text_uses_true_color() {
        assert_eq!(
            Paint::colored().color(Palette::MOCHA.blue, "lmx"),
            "\x1b[38;2;137;180;250mlmx\x1b[0m"
        );
        assert_eq!(Paint::colored().bold("x"), "\x1b[1mx\x1b[0m");
        assert_eq!(Paint::colored().color(Palette::MOCHA.yellow, ""), "");
    }

    #[test]
    fn takes_the_themes_colors_and_keeps_mocha_for_the_rest() {
        let theme = Theme {
            flavor: "latte".into(),
            palette: [
                ("blue", "#1e66f5"),
                ("peach", "not a color"),
                ("green", "#40A02B"),
            ]
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
            ansi: [("blue".to_owned(), Some(1))].into_iter().collect(),
        };
        let palette = Palette::of(&theme);
        assert_eq!(palette.blue, Color::Rgb(30, 102, 245));
        assert_eq!(palette.green, Color::Rgb(64, 160, 43));
        assert_eq!(palette.peach, Palette::MOCHA.peach);
        assert_eq!(palette.mauve, Palette::MOCHA.mauve);
        assert_eq!(
            Palette::of_config(&Err("unreadable".into())),
            Palette::MOCHA
        );
    }

    #[test]
    fn the_system_flavor_draws_in_the_terminals_colors() {
        let theme = Theme {
            flavor: "system".into(),
            palette: [("blue".to_owned(), "#89b4fa".to_owned())]
                .into_iter()
                .collect(),
            ansi: [
                ("blue", Some(12)),
                ("mauve", Some(5)),
                ("subtext0", None),
                ("red", Some(16)),
            ]
            .into_iter()
            .map(|(name, index)| (name.to_owned(), index))
            .collect(),
        };
        let palette = Palette::of(&theme);
        assert_eq!(palette.blue, Color::Terminal(12));
        assert_eq!(palette.mauve, Color::Terminal(5));
        assert_eq!(palette.subtext, Color::Default);
        assert_eq!(palette.red, Palette::TERMINAL.red);
        assert_eq!(palette.green, Palette::TERMINAL.green);

        let paint = Paint {
            enabled: true,
            palette,
        };
        assert_eq!(paint.color(palette.mauve, "lmx"), "\x1b[35mlmx\x1b[0m");
        assert_eq!(paint.color(palette.blue, "lmx"), "\x1b[94mlmx\x1b[0m");
        assert_eq!(paint.color(palette.subtext, "lmx"), "lmx");
        assert_eq!(Paint::plain().color(palette.blue, "lmx"), "lmx");
    }
}
