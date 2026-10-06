//! Text layout in terminal columns.
//!
//! Widths follow Unicode East Asian Width, so CJK and emoji take two columns and wrapped text never
//! runs past the edge of the screen.

use unicode_width::UnicodeWidthStr;

/// Terminal columns taken by `text`.
pub(crate) fn columns(text: &str) -> usize {
    text.width()
}

/// Splits `text` into lines of at most `width` columns.
///
/// A line breaks at its last space when it has one and never splits a character; spaces at a break
/// are dropped. A character wider than `width` gets a line of its own.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        // Prefixes are measured whole, as `columns` measures: joined emoji are narrower than the
        // sum of their characters.
        let end = rest
            .char_indices()
            .map(|(index, character)| index + character.len_utf8())
            .take_while(|end| columns(&rest[..*end]) <= width)
            .last()
            .unwrap_or_else(|| rest.chars().next().map_or(rest.len(), char::len_utf8));
        let mut line = &rest[..end];
        if end < rest.len()
            && !rest[end..].starts_with(' ')
            && let Some(space) = line.rfind(' ').filter(|space| *space > 0)
        {
            line = &line[..space];
        }
        rest = rest[line.len()..].trim_start_matches(' ');
        let line = line.trim_end_matches(' ');
        if !line.is_empty() {
            lines.push(line.to_owned());
        }
    }
    lines
}

/// Writes labeled rows: the label column is `label_width` wide, and every further line of a value
/// stays in the value column.
pub(crate) fn rows(rows: &[(&str, String)], label_width: usize) -> String {
    let mut text = String::new();
    for (label, value) in rows {
        let mut lines = value.lines();
        let first = lines.next().unwrap_or_default();
        let padding = label_width.saturating_sub(columns(label));
        text.push_str(&format!("{label}{:padding$}{first}\n", ""));
        for line in lines {
            text.push_str(&format!("{:label_width$}{line}\n", ""));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_wide_characters_as_two_columns() {
        assert_eq!(columns("lmx"), 3);
        assert_eq!(columns("界"), 2);
        assert_eq!(columns("Привет ✓"), 8);
    }

    #[test]
    fn breaks_at_the_last_space_within_the_width() {
        assert_eq!(
            wrap("lmx:console, lmx:git, lmx:go", 14),
            ["lmx:console,", "lmx:git,", "lmx:go"]
        );
        assert_eq!(wrap("a lmx:console b", 13), ["a lmx:console", "b"]);
        assert_eq!(wrap("aaaa  bbbb", 5), ["aaaa", "bbbb"]);
    }

    #[test]
    fn breaks_long_words_without_splitting_characters() {
        assert_eq!(wrap(&"界".repeat(5), 5), ["界界", "界界", "界"]);
        assert_eq!(wrap("/workspace/project", 10), ["/workspace", "/project"]);
    }

    #[test]
    fn keeps_continuation_lines_in_the_value_column() {
        let text = rows(
            &[
                ("Kernel", "Linux 6.12.5".into()),
                ("Failed units", "a.service\nb.service".into()),
            ],
            14,
        );
        assert_eq!(
            text,
            "Kernel        Linux 6.12.5\nFailed units  a.service\n              b.service\n"
        );
    }

    #[test]
    fn measures_joined_emoji_as_one_wide_character() {
        let developer = "👨\u{200d}💻";
        assert_eq!(columns(developer), 2);
        assert_eq!(
            wrap(&format!("{developer} {developer}"), 5),
            [format!("{developer} {developer}")]
        );
    }

    #[test]
    fn pads_labels_by_terminal_columns() {
        assert_eq!(rows(&[("界", "value".into())], 4), "界  value\n");
    }

    #[test]
    fn keeps_short_text_on_one_line() {
        assert_eq!(wrap("none", 67), ["none"]);
        assert!(wrap("", 67).is_empty());
    }
}
