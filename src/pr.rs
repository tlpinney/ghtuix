//! The PR view: one pull request in tabs, drawn as scrollable lines.

use crate::app::Action;
use crate::model::{Checks, Entry, Event, PrCommit, PrPage, PullRequest};
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
    Commits,
    Files,
    Checks,
    Timeline,
}

impl PrTab {
    pub const ALL: [(PrTab, char, &'static str); 6] = [
        (PrTab::Summary, 'o', "Summary"),
        (PrTab::Conversation, 'c', "Conversation"),
        (PrTab::Commits, 'h', "Commits"),
        (PrTab::Files, 'f', "Files"),
        (PrTab::Checks, 'k', "Checks"),
        (PrTab::Timeline, 't', "Timeline"),
    ];
}

/// A line the cursor can stop on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// The header of `page.threads[i]` (Conversation).
    Thread(usize),
    /// A diff line (Files).
    Line(crate::diff::Place),
    /// `page.checks[i]` (Checks).
    Check(usize),
}

pub struct PrView {
    /// The Inbox row it was opened from (shown until the page arrives).
    pub summary: PullRequest,
    pub page: Option<PrPage>,
    pub diff: Option<String>,
    /// Read when the Commits tab first shows.
    pub commits: Option<Result<Vec<PrCommit>, String>>,
    /// Read when the Timeline tab first shows.
    pub timeline: Option<Result<Vec<Event>, String>>,
    pub loading: bool,
    pub diff_loading: bool,
    pub error: Option<String>,
    pub tab: PrTab,
    pub scroll: u16,
    /// Index into the current tab's anchors.
    pub cursor: usize,
}

impl PrView {
    pub fn new(summary: PullRequest) -> Self {
        Self {
            summary,
            page: None,
            diff: None,
            commits: None,
            timeline: None,
            loading: true,
            diff_loading: false,
            error: None,
            tab: PrTab::Summary,
            scroll: 0,
            cursor: 0,
        }
    }

    /// The anchor under the cursor, if the tab has any.
    pub fn anchor(&self) -> Option<Anchor> {
        let (_, anchors) = self.content();
        anchors.get(self.cursor).map(|(_, anchor)| anchor.clone())
    }

    /// Moves the cursor between anchors; a tab without anchors scrolls instead.
    pub fn move_cursor(&mut self, delta: isize) {
        let count = self.content().1.len();
        if count == 0 {
            self.scroll_by(delta);
        } else {
            self.cursor = self.cursor.saturating_add_signed(delta).min(count - 1);
        }
    }

    pub fn switch(&mut self, tab: PrTab) {
        self.tab = tab;
        self.scroll = 0;
        self.cursor = 0;
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
        let mut hits = self.draw_tabs(frame, tabs);
        let (mut lines, anchors) = self.content();
        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(content);
        // The diff is not wrapped, so every diff line is one row and can be clicked.
        let wrap = self.tab != PrTab::Files;
        let width = usize::from(inner.width.max(1));
        let rows = |line: &Line| {
            if wrap {
                line.width().div_ceil(width).max(1)
            } else {
                1
            }
        };
        let mut starts = Vec::with_capacity(lines.len());
        let mut total = 0;
        for line in &lines {
            starts.push(total);
            total += rows(line);
        }
        self.cursor = self.cursor.min(anchors.len().saturating_sub(1));
        if let Some((line, _)) = anchors.get(self.cursor) {
            // Keep the cursor's line on screen and mark it.
            let row = starts[*line] as u16;
            if row < self.scroll {
                self.scroll = row;
            } else if row >= self.scroll + inner.height {
                self.scroll = row + 1 - inner.height;
            }
            lines[*line] = lines[*line]
                .clone()
                .style(Style::default().add_modifier(Modifier::REVERSED));
        }
        self.scroll = self.scroll.min((total as u16).saturating_sub(inner.height));
        for (index, (line, _)) in anchors.iter().enumerate() {
            let row = starts[*line] as u16;
            if (self.scroll..self.scroll + inner.height).contains(&row) {
                let rect = Rect::new(inner.x, inner.y + row - self.scroll, inner.width, 1);
                hits.push((rect, Action::Cursor(index)));
            }
        }
        let paragraph = Paragraph::new(lines).scroll((self.scroll, 0));
        let paragraph = if wrap {
            paragraph.wrap(Wrap { trim: false })
        } else {
            paragraph
        };
        frame.render_widget(paragraph.block(block), content);
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

    /// The current tab's lines.
    #[cfg(test)]
    pub fn lines(&self) -> Vec<Line<'static>> {
        self.content().0
    }

    /// The current tab's lines and the anchors among them (`(line index, anchor)`).
    pub fn content(&self) -> (Vec<Line<'static>>, Vec<(usize, Anchor)>) {
        if let Some(error) = &self.error {
            let lines = vec![Line::raw(error.clone()), Line::raw("Press r to try again.")];
            return (lines, Vec::new());
        }
        let Some(page) = &self.page else {
            return (vec![Line::raw("Loading…")], Vec::new());
        };
        match self.tab {
            PrTab::Summary => (summary(page), Vec::new()),
            PrTab::Conversation => conversation(page),
            PrTab::Files => files(page, self.diff.as_deref(), self.diff_loading),
            PrTab::Checks => checks(page),
            PrTab::Commits => (commits(self.commits.as_ref()), Vec::new()),
            PrTab::Timeline => (timeline(self.timeline.as_ref()), Vec::new()),
        }
    }
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
    lines.extend(crate::markdown::lines(&page.body, ""));
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
    lines.extend(crate::markdown::lines(&entry.body, &format!("{indent}  ")));
    lines.push(Line::raw(""));
}

fn conversation(page: &PrPage) -> (Vec<Line<'static>>, Vec<(usize, Anchor)>) {
    if page.conversation.is_empty() && page.threads.is_empty() {
        return (vec![Line::raw("No comments or reviews yet")], Vec::new());
    }
    let mut lines = Vec::new();
    let mut anchors = Vec::new();
    for item in &page.conversation {
        entry(&mut lines, item, "");
    }
    if !page.threads.is_empty() {
        lines.push(bold(format!("REVIEW THREADS ({})", page.threads.len())));
        lines.push(Line::raw(""));
    }
    for (index, thread) in page.threads.iter().enumerate() {
        let line = thread.line.map_or(String::new(), |line| format!(":{line}"));
        let resolved = if thread.resolved { " · resolved" } else { "" };
        anchors.push((lines.len(), Anchor::Thread(index)));
        lines.push(bold(format!("{}{line}{resolved}", thread.path)));
        for comment in &thread.comments {
            entry(&mut lines, comment, "  ");
        }
    }
    (lines, anchors)
}

fn files(
    page: &PrPage,
    diff: Option<&str>,
    loading: bool,
) -> (Vec<Line<'static>>, Vec<(usize, Anchor)>) {
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
    let mut anchors = Vec::new();
    match diff {
        Some(diff) => {
            let start = lines.len();
            for (offset, place) in crate::diff::places(diff).into_iter().enumerate() {
                if let Some(place) = place {
                    anchors.push((start + offset, Anchor::Line(place)));
                }
            }
            lines.extend(crate::diff::lines(diff));
        }
        None if loading => lines.push(Line::raw("Loading the diff…")),
        None => lines.push(Line::raw("The diff is not loaded. Press r to load it.")),
    }
    (lines, anchors)
}

fn commits(commits: Option<&Result<Vec<PrCommit>, String>>) -> Vec<Line<'static>> {
    let commits = match commits {
        None => return vec![Line::raw("Loading commits…")],
        Some(Err(error)) => return vec![Line::raw(error.clone())],
        Some(Ok(commits)) => commits,
    };
    let mut lines = vec![bold(format!("{} commits, oldest first", commits.len()))];
    for commit in commits {
        let (mark, color) = check_mark(commit.checks);
        lines.push(Line::from(vec![
            Span::styled(format!("{mark} "), Style::default().fg(color)),
            Span::styled(
                commit.oid.chars().take(8).collect::<String>(),
                Style::default().fg(Color::Magenta),
            ),
            Span::raw(format!(" {}", commit.headline)),
            Span::styled(
                format!("  {} {}", commit.author, time::age(&commit.date)),
                Style::default().add_modifier(Modifier::DIM),
            ),
        ]));
    }
    lines
}

fn timeline(events: Option<&Result<Vec<Event>, String>>) -> Vec<Line<'static>> {
    let events = match events {
        None => return vec![Line::raw("Loading the timeline…")],
        Some(Err(error)) => return vec![Line::raw(error.clone())],
        Some(Ok(events)) => events,
    };
    if events.is_empty() {
        return vec![Line::raw("Nothing has happened yet")];
    }
    events
        .iter()
        .map(|event| {
            Line::from(vec![
                Span::styled(
                    format!("{:>4} ", time::age(&event.at)),
                    Style::default().add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    event.actor.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(" {}", event.text)),
            ])
        })
        .collect()
}

fn check_mark(state: Checks) -> (&'static str, Color) {
    match state {
        Checks::Passing => ("✓", Color::Green),
        Checks::Failing => ("✗", Color::Red),
        Checks::Pending => ("•", Color::Yellow),
        Checks::None => (" ", Color::Reset),
    }
}

fn checks(page: &PrPage) -> (Vec<Line<'static>>, Vec<(usize, Anchor)>) {
    if page.checks.is_empty() {
        return (vec![Line::raw("No checks on the head commit")], Vec::new());
    }
    let anchors = (0..page.checks.len())
        .map(|i| (i, Anchor::Check(i)))
        .collect();
    let lines = page
        .checks
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
        .collect();
    (lines, anchors)
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
            id: "PR_1".into(),
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
            merge_state: "clean".into(),
            merge_methods: vec![
                crate::model::MergeMethod::Merge,
                crate::model::MergeMethod::Squash,
            ],
            decision: Decision::None,
            conversation: vec![Entry {
                author: person("coderabbitai", true),
                body: "Looks fine".into(),
                at: "2026-10-27T10:30:00Z".into(),
                review: Some(Stance::Reviewed),
            }],
            threads: vec![Thread {
                id: "T_1".into(),
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
                job_id: Some(42),
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
