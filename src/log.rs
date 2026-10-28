//! A GitHub Actions job log: timestamps trimmed, groups as headings, failures marked, and
//! jumps between failures. Opens at the first failure.

use crate::app::Action;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Group,
    Warning,
    Failure,
}

pub struct LogView {
    pub title: String,
    /// The job's page on GitHub, for `w`.
    pub url: Option<String>,
    pub job_id: i64,
    pub lines: Option<Result<Vec<(String, Kind)>, String>>,
    pub scroll: usize,
    /// The failure the view last jumped to (an index into `lines`).
    pub at: Option<usize>,
}

impl LogView {
    pub fn new(title: String, url: Option<String>, job_id: i64) -> Self {
        Self {
            title,
            url,
            job_id,
            lines: None,
            scroll: 0,
            at: None,
        }
    }

    /// Takes the downloaded log and moves to its first failure.
    pub fn load(&mut self, result: Result<String, String>) {
        self.lines = Some(result.map(|text| parse(&text)));
        self.scroll = 0;
        self.at = None;
        self.jump(1);
    }

    fn failures(&self) -> Vec<usize> {
        match &self.lines {
            Some(Ok(lines)) => (0..lines.len())
                .filter(|i| lines[*i].1 == Kind::Failure)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Moves to the next (`1`) or previous (`-1`) failure, a few lines below the top.
    pub fn jump(&mut self, direction: isize) {
        let failures = self.failures();
        let next = match (self.at, direction > 0) {
            (None, _) => failures.first(),
            (Some(at), true) => failures.iter().find(|i| **i > at),
            (Some(at), false) => failures.iter().rev().find(|i| **i < at),
        };
        if let Some(index) = next.copied() {
            self.at = Some(index);
            self.scroll = index.saturating_sub(3);
        }
    }

    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = self.scroll.saturating_add_signed(delta);
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let [title, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
        let failures = self.failures();
        let summary = match failures.len() {
            0 => "no failures found".to_string(),
            n => {
                let at = self
                    .at
                    .and_then(|at| failures.iter().position(|i| *i == at))
                    .map_or(0, |i| i + 1);
                format!("failure {at} of {n}")
            }
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" {} ", self.title),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("• {summary}")),
            ])),
            title,
        );
        let block = Block::default().borders(Borders::ALL).title(" Log ");
        let inner = block.inner(body);
        frame.render_widget(block, body);
        let lines = match &self.lines {
            None => vec![Line::raw("Loading the log…")],
            Some(Err(error)) => vec![Line::raw(error.clone())],
            Some(Ok(lines)) => {
                let height = usize::from(inner.height);
                self.scroll = self.scroll.min(lines.len().saturating_sub(height));
                lines
                    .iter()
                    .enumerate()
                    .skip(self.scroll)
                    .take(height)
                    .map(|(index, (text, kind))| {
                        let mut style = match kind {
                            Kind::Plain => Style::default(),
                            Kind::Group => Style::default().add_modifier(Modifier::BOLD),
                            Kind::Warning => Style::default().fg(Color::Yellow),
                            Kind::Failure => Style::default().fg(Color::Red),
                        };
                        if Some(index) == self.at {
                            style = style.add_modifier(Modifier::REVERSED);
                        }
                        Line::styled(text.clone(), style)
                    })
                    .collect()
            }
        };
        frame.render_widget(Paragraph::new(lines), inner);
        Vec::new()
    }
}

/// Splits a raw log into lines with their kind.
pub fn parse(text: &str) -> Vec<(String, Kind)> {
    let mut out = Vec::new();
    for raw in text.lines() {
        // GitHub starts a log with a byte-order mark.
        let line = trim_timestamp(raw.trim_start_matches('\u{feff}').trim_end_matches('\r'));
        if line.starts_with("##[endgroup]") {
            continue;
        }
        let (text, kind) = if let Some(rest) = line.strip_prefix("##[group]") {
            (format!("▸ {rest}"), Kind::Group)
        } else if let Some(rest) = line.strip_prefix("##[error]") {
            (rest.to_owned(), Kind::Failure)
        } else if let Some(rest) = line.strip_prefix("##[warning]") {
            (rest.to_owned(), Kind::Warning)
        } else if failure_text(line) {
            (line.to_owned(), Kind::Failure)
        } else {
            (line.to_owned(), Kind::Plain)
        };
        out.push((text.replace('\t', "    "), kind));
    }
    out
}

/// Common compiler and test runner failure markers.
fn failure_text(line: &str) -> bool {
    let lower = line.trim_start();
    lower.starts_with("error:")
        || lower.starts_with("error[")
        || lower.starts_with("FAILED")
        || lower.contains(" FAILED")
        || lower.contains("panicked at")
        || lower.starts_with("Error:")
}

/// Drops the `2026-10-27T22:52:57.1234567Z ` prefix GitHub puts on every line.
fn trim_timestamp(line: &str) -> &str {
    let bytes = line.as_bytes();
    let stamped = bytes.len() > 20
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[10] == b'T';
    match line.find("Z ") {
        Some(end) if stamped && end < 40 => &line[end + 2..],
        _ => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "2026-10-27T22:00:00.1234567Z ##[group]Run cargo test\n\
                       2026-10-27T22:00:01.0000000Z running 3 tests\n\
                       2026-10-27T22:00:02.0000000Z ##[endgroup]\n\
                       2026-10-27T22:00:03.0000000Z test a ... FAILED\n\
                       2026-10-27T22:00:04.0000000Z ##[warning]slow\n\
                       2026-10-27T22:00:05.0000000Z ##[error]Process completed with exit code 101.";

    #[test]
    fn parses_groups_failures_and_trims_timestamps() {
        let lines = parse(LOG);
        assert_eq!(lines[0], ("▸ Run cargo test".to_string(), Kind::Group));
        assert_eq!(lines[1], ("running 3 tests".to_string(), Kind::Plain));
        assert_eq!(lines[2].1, Kind::Failure, "endgroup lines are dropped");
        assert_eq!(lines[3].1, Kind::Warning);
        assert_eq!(lines[4].0, "Process completed with exit code 101.");
        assert_eq!(trim_timestamp("plain line"), "plain line");
        let bom = parse("\u{feff}2026-10-28T03:08:18.1848113Z Current runner version");
        assert_eq!(bom[0].0, "Current runner version");
    }

    #[test]
    fn opens_at_the_first_failure_and_jumps_both_ways() {
        let mut view = LogView::new("test".into(), None, 1);
        view.load(Ok(LOG.into()));
        assert_eq!(view.at, Some(2));
        view.jump(1);
        assert_eq!(view.at, Some(4));
        view.jump(1);
        assert_eq!(view.at, Some(4), "stays on the last failure");
        view.jump(-1);
        assert_eq!(view.at, Some(2));
    }
}
