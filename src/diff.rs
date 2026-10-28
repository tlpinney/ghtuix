//! Unified diffs and `git show` output as coloured lines.

use crate::model::Side;
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

/// The place a diff line comments on: its file, line number and side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub path: String,
    pub line: i64,
    pub side: Side,
}

/// For every line of a unified diff, the place it is (`None` for headers and markers).
/// Added and context lines are on the new side; removed lines on the old side.
pub fn places(diff: &str) -> Vec<Option<Place>> {
    let (mut path, mut old, mut new) = (String::new(), 0i64, 0i64);
    let mut in_hunk = false;
    let mut out = Vec::new();
    for text in diff.lines() {
        let place = |line: i64, side: Side| {
            Some(Place {
                path: path.clone(),
                line,
                side,
            })
        };
        let here = if text.starts_with("diff --git ") {
            in_hunk = false;
            path = text
                .rsplit_once(" b/")
                .map_or(String::new(), |(_, p)| p.to_owned());
            None
        } else if let Some(hunk) = text.strip_prefix("@@ ") {
            // `@@ -a[,b] +c[,d] @@`
            let mut numbers = hunk.split_whitespace().take(2).map(|part| {
                part[1..]
                    .split(',')
                    .next()
                    .and_then(|n| n.parse::<i64>().ok())
                    .unwrap_or(1)
            });
            old = numbers.next().unwrap_or(1);
            new = numbers.next().unwrap_or(1);
            in_hunk = true;
            None
        } else if !in_hunk {
            if let Some(p) = text.strip_prefix("+++ b/") {
                path = p.to_owned();
            }
            None
        } else if text.starts_with('+') {
            new += 1;
            place(new - 1, Side::Right)
        } else if text.starts_with('-') {
            old += 1;
            place(old - 1, Side::Left)
        } else if text.starts_with(' ') || text.is_empty() {
            old += 1;
            new += 1;
            place(new - 1, Side::Right)
        } else {
            None
        };
        out.push(here);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_follow_hunks_sides_and_files() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n\
                    +++ b/src/a.rs\n@@ -10,3 +10,3 @@ fn x()\n keep\n-old\n+new\n\\ No newline\n\
                    diff --git a/b.txt b/b.txt\n@@ -0,0 +1 @@\n+first";
        let places = places(diff);
        let at = |path: &str, line, side| {
            Some(Place {
                path: path.into(),
                line,
                side,
            })
        };
        assert_eq!(places[..5], [None, None, None, None, None]);
        assert_eq!(places[5], at("src/a.rs", 10, Side::Right));
        assert_eq!(places[6], at("src/a.rs", 11, Side::Left));
        assert_eq!(places[7], at("src/a.rs", 11, Side::Right));
        assert_eq!(places[8], None, "the no-newline marker is not a line");
        assert_eq!(places[11], at("b.txt", 1, Side::Right));
    }

    #[test]
    fn colours_follow_the_line_kind() {
        assert_eq!(line("+added").style.fg, Some(Color::Green));
        assert_eq!(line("-removed").style.fg, Some(Color::Red));
        assert_eq!(line("+++ b/x").style.fg, None);
        assert_eq!(line("@@ -1 +1 @@").style.fg, Some(Color::Cyan));
        assert_eq!(line("\tindented").to_string(), "    indented");
    }
}
