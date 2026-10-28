//! Unified diffs and `git show` output as coloured lines.

use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
};

/// Every line of `diff`, styled by what it is.
pub fn lines(diff: &str) -> Vec<Line<'static>> {
    diff.lines().map(line).collect()
}

/// One line: file headers bold, hunk headers cyan, additions green, removals red.
pub fn line(text: &str) -> Line<'static> {
    let style = if text.starts_with("diff --git") || text.starts_with("commit ") {
        Style::default().add_modifier(Modifier::BOLD)
    } else if text.starts_with("@@") {
        Style::default().fg(Color::Cyan)
    } else if text.starts_with('+') && !text.starts_with("+++") {
        Style::default().fg(Color::Green)
    } else if text.starts_with('-') && !text.starts_with("---") {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    };
    Line::styled(text.replace('\t', "    "), style)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_follow_the_line_kind() {
        assert_eq!(line("+added").style.fg, Some(Color::Green));
        assert_eq!(line("-removed").style.fg, Some(Color::Red));
        assert_eq!(line("+++ b/x").style.fg, None);
        assert_eq!(line("@@ -1 +1 @@").style.fg, Some(Color::Cyan));
        assert_eq!(line("\tindented").to_string(), "    indented");
    }
}
