//! The PR view: one pull request in tabs, drawn as scrollable lines.

use crate::app::Action;
use crate::model::{Checks, Entry, PrPage, PullRequest};
use crate::time;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrTab {
    Summary,
    Conversation,
    Files,
    Checks,
}

impl PrTab {
    pub const ALL: [(PrTab, char, &'static str); 4] = [
        (PrTab::Summary, 'o', "Summary"),
        (PrTab::Conversation, 'c', "Conversation"),
        (PrTab::Files, 'f', "Files"),
        (PrTab::Checks, 'k', "Checks"),
    ];
}

pub struct PrView {
    /// The Inbox row it was opened from (shown until the page arrives).
    pub summary: PullRequest,
    pub page: Option<PrPage>,
    pub diff: Option<String>,
    pub loading: bool,
    pub diff_loading: bool,
    pub error: Option<String>,
    pub tab: PrTab,
    pub scroll: u16,
}

impl PrView {
    pub fn new(summary: PullRequest) -> Self {
        Self {
            summary,
            page: None,
            diff: None,
            loading: true,
            diff_loading: false,
            error: None,
            tab: PrTab::Summary,
            scroll: 0,
        }
    }

    pub fn url(&self) -> &str {
        self.page
            .as_ref()
            .map_or(&self.summary.url, |page| &page.url)
    }

    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = self.scroll.saturating_add_signed(delta as i16);
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let [title, tabs, content] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .areas(area);
        let state = self.page.as_ref().map_or(String::new(), |page| {
            let draft = if page.draft { " · draft" } else { "" };
            format!(" • {}{draft}", page.state)
        });
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" {} ", self.summary.reference()),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{}{state}", self.summary.title)),
            ])),
            title,
        );
        let hits = self.draw_tabs(frame, tabs);
        let lines = self.lines();
        let block = Block::default().borders(Borders::ALL);
        let visible = block.inner(content).height;
        let max = (lines.len() as u16).saturating_sub(visible);
        self.scroll = self.scroll.min(max);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0))
                .block(block),
            content,
        );
        hits
    }

    fn draw_tabs(&self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let mut spans = Vec::new();
        let mut hits = Vec::new();
        let mut x = area.x + 1;
        for (tab, key, label) in PrTab::ALL {
            let text = format!("{key} {label}");
            let width = text.chars().count() as u16;
            let style = if tab == self.tab {
                Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default()
            };
            spans.push(Span::styled(text, style));
            spans.push(Span::raw("   "));
            hits.push((Rect::new(x, area.y, width, 1), Action::PrTab(tab)));
            x += width + 3;
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                x: area.x + 1,
                ..area
            },
        );
        hits
    }

    /// The current tab's content.
    pub fn lines(&self) -> Vec<Line<'static>> {
        if let Some(error) = &self.error {
            return vec![Line::raw(error.clone()), Line::raw("Press r to try again.")];
        }
        let Some(page) = &self.page else {
            return vec![Line::raw("Loading…")];
        };
        match self.tab {
            PrTab::Summary => summary(page),
            PrTab::Conversation => conversation(page),
            PrTab::Files => files(page, self.diff.as_deref(), self.diff_loading),
            PrTab::Checks => checks(page),
        }
    }
}

/// A body's lines, trimmed at the end, with runs of blank lines shown as one.
fn body_lines(body: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in body.lines().map(str::trim_end) {
        if line.is_empty() && lines.last().is_none_or(|last| last.is_empty()) {
            continue;
        }
        lines.push(line.to_owned());
    }
    while lines.last().is_some_and(|last| last.is_empty()) {
        lines.pop();
    }
    lines
}

fn bold(text: impl Into<String>) -> Line<'static> {
    Line::styled(text.into(), Style::default().add_modifier(Modifier::BOLD))
}

fn bot(is_bot: bool) -> &'static str {
    if is_bot { " [bot]" } else { "" }
}

fn summary(page: &PrPage) -> Vec<Line<'static>> {
    let passing = page
        .checks
        .iter()
        .filter(|c| c.state == Checks::Passing)
        .count();
    let mut lines = vec![
        bold(page.title.clone()),
        Line::raw(""),
        Line::raw(format!(
            "Author     {}{}",
            page.author.login,
            bot(page.author.bot)
        )),
        Line::raw(format!("Branches   {} → {}", page.head_ref, page.base_ref)),
        Line::raw(format!(
            "Changes    +{} −{} in {} files",
            page.additions, page.deletions, page.changed_files
        )),
        Line::raw(format!("Mergeable  {}", page.mergeable)),
        Line::raw(format!(
            "Checks     {passing}/{} passing",
            page.checks.len()
        )),
        Line::raw(format!("Head       {}", page.head_oid)),
        Line::raw(format!("Base       {}", page.base_oid)),
        Line::raw(format!(
            "Opened     {} ({} ago)",
            page.created_at,
            time::age(&page.created_at)
        )),
        Line::raw(format!("Updated    {}", page.updated_at)),
        Line::raw(format!("Link       {}", page.url)),
        Line::raw(""),
        bold("DESCRIPTION"),
    ];
    if page.body.trim().is_empty() {
        lines.push(Line::raw("No description"));
    }
    lines.extend(body_lines(&page.body).into_iter().map(Line::raw));
    lines
}

fn entry(lines: &mut Vec<Line<'static>>, entry: &Entry, indent: &str) {
    let what = entry.review.map_or("commented", |stance| stance.label());
    lines.push(Line::from(vec![
        Span::styled(
            format!("{indent}{}{}", entry.author.login, bot(entry.author.bot)),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" · {what} · {} ago", time::age(&entry.at))),
    ]));
    for line in body_lines(&entry.body) {
        lines.push(Line::raw(format!("{indent}  {line}")));
    }
    lines.push(Line::raw(""));
}

fn conversation(page: &PrPage) -> Vec<Line<'static>> {
    if page.conversation.is_empty() && page.threads.is_empty() {
        return vec![Line::raw("No comments or reviews yet")];
    }
    let mut lines = Vec::new();
    for item in &page.conversation {
        entry(&mut lines, item, "");
    }
    if !page.threads.is_empty() {
        lines.push(bold(format!("REVIEW THREADS ({})", page.threads.len())));
        lines.push(Line::raw(""));
    }
    for thread in &page.threads {
        let line = thread.line.map_or(String::new(), |line| format!(":{line}"));
        let resolved = if thread.resolved { " · resolved" } else { "" };
        lines.push(bold(format!("{}{line}{resolved}", thread.path)));
        for comment in &thread.comments {
            entry(&mut lines, comment, "  ");
        }
    }
    lines
}

fn files(page: &PrPage, diff: Option<&str>, loading: bool) -> Vec<Line<'static>> {
    let mut lines = vec![bold(format!(
        "{} files · +{} −{}",
        page.changed_files, page.additions, page.deletions
    ))];
    for file in &page.files {
        lines.push(Line::raw(format!(
            "  +{:<5} −{:<5} {} ({})",
            file.additions, file.deletions, file.path, file.change
        )));
    }
    lines.push(Line::raw(""));
    match diff {
        Some(diff) => lines.extend(diff.lines().map(diff_line)),
        None if loading => lines.push(Line::raw("Loading the diff…")),
        None => lines.push(Line::raw("The diff is not loaded. Press r to load it.")),
    }
    lines
}

/// One diff line, coloured by what it is.
fn diff_line(line: &str) -> Line<'static> {
    let style = if line.starts_with("diff --git") {
        Style::default().add_modifier(Modifier::BOLD)
    } else if line.starts_with("@@") {
        Style::default().fg(Color::Cyan)
    } else if line.starts_with('+') && !line.starts_with("+++") {
        Style::default().fg(Color::Green)
    } else if line.starts_with('-') && !line.starts_with("---") {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    };
    Line::styled(line.replace('\t', "    "), style)
}

fn checks(page: &PrPage) -> Vec<Line<'static>> {
    if page.checks.is_empty() {
        return vec![Line::raw("No checks on the head commit")];
    }
    page.checks
        .iter()
        .map(|check| {
            let (mark, color) = match check.state {
                Checks::Passing => ("✓", Color::Green),
                Checks::Failing => ("✗", Color::Red),
                Checks::Pending | Checks::None => ("•", Color::Yellow),
            };
            Line::from(vec![
                Span::styled(format!("{mark} "), Style::default().fg(color)),
                Span::raw(format!("{} — {}", check.name, check.detail)),
            ])
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Check, Decision, FileChange, Person, Stance, Thread};

    pub(crate) fn page() -> PrPage {
        let person = |login: &str, bot| Person {
            login: login.into(),
            bot,
        };
        PrPage {
            repo: "o/a".into(),
            number: 1,
            title: "Change 1".into(),
            url: "https://github.com/o/a/pull/1".into(),
            body: "Why".into(),
            state: "open".into(),
            draft: false,
            author: person("alice", false),
            created_at: "2026-10-27T10:00:00Z".into(),
            updated_at: "2026-10-27T11:00:00Z".into(),
            head_ref: "feature".into(),
            base_ref: "main".into(),
            head_oid: "a".repeat(40),
            base_oid: "b".repeat(40),
            additions: 3,
            deletions: 1,
            changed_files: 1,
            mergeable: "mergeable".into(),
            decision: Decision::None,
            conversation: vec![Entry {
                author: person("coderabbitai", true),
                body: "Looks fine".into(),
                at: "2026-10-27T10:30:00Z".into(),
                review: Some(Stance::Reviewed),
            }],
            threads: vec![Thread {
                path: "src/x.rs".into(),
                line: Some(7),
                resolved: false,
                comments: vec![],
            }],
            files: vec![FileChange {
                path: "src/x.rs".into(),
                additions: 3,
                deletions: 1,
                change: "modified".into(),
            }],
            checks: vec![Check {
                name: "CodeRabbit".into(),
                state: Checks::Passing,
                detail: "success".into(),
                url: None,
            }],
        }
    }

    fn text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn bodies_collapse_blank_runs_and_trailing_space() {
        assert_eq!(body_lines("a  \n\n\n\nb\n\n"), ["a", "", "b"]);
        assert!(body_lines("\n\n").is_empty());
    }

    #[test]
    fn each_tab_shows_its_part_of_the_page() {
        let mut view = PrView::new(crate::inbox::tests::pr(1, "o/a", &[]));
        assert!(text(&view.lines()).contains("Loading"));
        view.page = Some(page());
        assert!(text(&view.lines()).contains("feature → main"));
        view.tab = PrTab::Conversation;
        let conversation = text(&view.lines());
        assert!(
            conversation.contains("coderabbitai [bot] · reviewed"),
            "{conversation}"
        );
        assert!(conversation.contains("src/x.rs:7"));
        view.tab = PrTab::Files;
        assert!(text(&view.lines()).contains("not loaded"));
        view.diff = Some("diff --git a/x b/x\n+new\n-old".into());
        assert!(text(&view.lines()).contains("+new"));
        view.tab = PrTab::Checks;
        assert!(text(&view.lines()).contains("✓ CodeRabbit — success"));
    }
}
