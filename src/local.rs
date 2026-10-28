//! The Local view: the current checkout's history, working changes and staged changes.

use crate::app::Action;
use crate::git::{GraphLine, Repo, Status};
use crate::time;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalTab {
    History,
    Working,
    Staged,
}

impl LocalTab {
    pub const ALL: [(LocalTab, char, &'static str); 3] = [
        (LocalTab::History, 'g', "History"),
        (LocalTab::Working, 'w', "Working"),
        (LocalTab::Staged, 'i', "Staged"),
    ];
}

/// The checkout's status and both diffs, read together.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub status: Status,
    pub working: String,
    pub staged: String,
    /// Who a commit would be recorded as (`Name <email>`).
    pub author: String,
}

pub struct Local {
    /// The checkout ghtuix was started in, or why there is none.
    pub repo: Result<Repo, String>,
    pub tab: LocalTab,
    pub history: Option<Result<Vec<GraphLine>, String>>,
    pub changes: Option<Result<Changes, String>>,
    /// Index into `history` of the selected line (always a commit line).
    pub selected: usize,
    /// The commit shown in full: its id and, once read, its `git show` output.
    pub commit: Option<(String, Option<String>)>,
    pub scroll: u16,
    pub loading: bool,
    /// The selected file of the Working or Staged list.
    pub file: usize,
    /// The cursor in the selected file's diff (an index into its lines, always on an
    /// added or removed line), while hunks and lines are being staged.
    pub line: Option<usize>,
    /// A local write is running; another waits so it never applies to stale text.
    pub writing: bool,
}

impl Local {
    pub fn new(repo: Result<Repo, String>) -> Self {
        Self {
            repo,
            tab: LocalTab::History,
            history: None,
            changes: None,
            selected: 0,
            commit: None,
            scroll: 0,
            loading: false,
            file: 0,
            line: None,
            writing: false,
        }
    }

    /// The selected file's own diff in the current tab, as read.
    pub fn file_diff(&self) -> Option<String> {
        let Some(Ok(changes)) = &self.changes else {
            return None;
        };
        let (path, _) = self.selected_file()?;
        let diff = match self.tab {
            LocalTab::Staged => &changes.staged,
            _ => &changes.working,
        };
        Some(crate::git::diff_of(diff, path)).filter(|one| !one.is_empty())
    }

    /// The changed lines of the selected file's diff, in order.
    fn change_lines(&self) -> Vec<usize> {
        let Some(diff) = self.file_diff() else {
            return Vec::new();
        };
        let Some(parsed) = crate::patch::FileDiff::parse(&diff) else {
            return Vec::new();
        };
        (0..diff.lines().count())
            .filter(|&i| parsed.is_change(i))
            .collect()
    }

    /// Puts the cursor on the first change of the selected file; false when it has none
    /// (a new or binary file is staged whole).
    pub fn enter_diff(&mut self) -> bool {
        self.line = self.change_lines().first().copied();
        self.line.is_some()
    }

    /// Moves the cursor `delta` changed lines on.
    pub fn move_line(&mut self, delta: isize) {
        let changes = self.change_lines();
        let Some(line) = self.line else { return };
        let at = changes
            .iter()
            .position(|&i| i >= line)
            .unwrap_or(changes.len().saturating_sub(1));
        self.line = changes
            .get(
                at.saturating_add_signed(delta)
                    .min(changes.len().saturating_sub(1)),
            )
            .copied();
    }

    /// Puts the cursor on diff line `index` if it is a change.
    pub fn place_line(&mut self, index: usize) {
        if self.change_lines().contains(&index) {
            self.line = Some(index);
        }
    }

    /// After the diff was read again, keeps the cursor on the nearest change, if any.
    pub fn settle_line(&mut self) {
        let Some(line) = self.line else { return };
        let changes = self.change_lines();
        self.line = changes
            .iter()
            .copied()
            .find(|&i| i >= line)
            .or_else(|| changes.last().copied());
    }

    /// The row of `lines()` where the selected file's diff starts.
    fn diff_row(&self) -> usize {
        2 + self.files().len() + 1
    }

    fn commit_lines(&self) -> Vec<usize> {
        match &self.history {
            Some(Ok(lines)) => (0..lines.len())
                .filter(|i| lines[*i].commit.is_some())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Moves the selection by `delta` commits (graph-only lines are skipped).
    pub fn move_selection(&mut self, delta: isize) {
        let commits = self.commit_lines();
        let Some(at) = commits.iter().position(|i| *i >= self.selected) else {
            self.selected = commits.first().copied().unwrap_or(0);
            return;
        };
        let to = at
            .saturating_add_signed(delta)
            .min(commits.len().saturating_sub(1));
        self.selected = commits[to];
    }

    pub fn selected_sha(&self) -> Option<String> {
        match &self.history {
            Some(Ok(lines)) => Some(lines.get(self.selected)?.commit.as_ref()?.sha.clone()),
            _ => None,
        }
    }

    /// The files the Working or Staged tab lists.
    pub fn files(&self) -> &[(String, char)] {
        match &self.changes {
            Some(Ok(changes)) if self.tab == LocalTab::Staged => &changes.status.staged,
            Some(Ok(changes)) => &changes.status.unstaged,
            _ => &[],
        }
    }

    pub fn selected_file(&self) -> Option<&(String, char)> {
        self.files().get(self.file)
    }

    /// Selects file `index`, showing its diff from the top.
    pub fn select_file(&mut self, index: usize) {
        self.file = index.min(self.files().len().saturating_sub(1));
        self.scroll = 0;
        self.line = None;
    }

    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = self.scroll.saturating_add_signed(delta as i16);
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let [tabs, content] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
        let mut hits = self.draw_tabs(frame, tabs);
        let root = match &self.repo {
            Ok(repo) => repo.root.display().to_string(),
            Err(error) => {
                frame.render_widget(
                    Paragraph::new(error.clone())
                        .block(Block::default().borders(Borders::ALL).title(" Local ")),
                    content,
                );
                return hits;
            }
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {root} "));
        let inner = block.inner(content);
        frame.render_widget(block, content);
        if self.tab == LocalTab::History && self.commit.is_none() {
            hits.extend(self.draw_history(frame, inner));
            return hits;
        }
        let lines = self.lines();
        // Keep the diff cursor in sight.
        if let Some(line) = self.line {
            let row = (self.diff_row() + line) as u16;
            if row < self.scroll {
                self.scroll = row;
            } else if row >= self.scroll + inner.height {
                self.scroll = row + 1 - inner.height;
            }
        }
        self.scroll = self
            .scroll
            .min((lines.len() as u16).saturating_sub(inner.height));
        // File rows start two lines down (under the branch line and a blank).
        if self.commit.is_none() {
            for index in 0..self.files().len() {
                let row = (2 + index) as u16;
                if (self.scroll..self.scroll + inner.height).contains(&row) {
                    let rect = Rect::new(inner.x, inner.y + row - self.scroll, inner.width, 1);
                    hits.push((rect, Action::Select(index)));
                }
            }
            // Each changed line takes the cursor when clicked.
            for index in self.change_lines() {
                let row = (self.diff_row() + index) as u16;
                if (self.scroll..self.scroll + inner.height).contains(&row) {
                    let rect = Rect::new(inner.x, inner.y + row - self.scroll, inner.width, 1);
                    hits.push((rect, Action::DiffLine(index)));
                }
            }
        }
        // A diff is not wrapped, so each of its lines stays one row (and one click target).
        let paragraph = Paragraph::new(lines).scroll((self.scroll, 0));
        let paragraph = if self.commit.is_some() {
            paragraph.wrap(Wrap { trim: false })
        } else {
            paragraph
        };
        frame.render_widget(paragraph, inner);
        hits
    }

    fn draw_tabs(&self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let mut spans = Vec::new();
        let mut hits = Vec::new();
        let mut x = area.x + 1;
        for (tab, key, label) in LocalTab::ALL {
            let text = format!("{key} {label}");
            let width = text.chars().count() as u16;
            let style = if tab == self.tab {
                Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default()
            };
            spans.push(Span::styled(text, style));
            spans.push(Span::raw("   "));
            hits.push((Rect::new(x, area.y, width, 1), Action::LocalTab(tab)));
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

    fn draw_history(&self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let lines = match &self.history {
            Some(Ok(lines)) => lines,
            Some(Err(error)) => {
                frame.render_widget(Paragraph::new(error.clone()), area);
                return Vec::new();
            }
            None => {
                frame.render_widget(Paragraph::new("Loading history…"), area);
                return Vec::new();
            }
        };
        let rows = area.height as usize;
        let top = self.selected.saturating_sub(rows.saturating_sub(1));
        let mut drawn = Vec::new();
        let mut hits = Vec::new();
        for (index, line) in lines.iter().enumerate().skip(top).take(rows) {
            let mut spans = vec![Span::styled(
                line.graph.clone(),
                Style::default().fg(Color::Yellow),
            )];
            if let Some(commit) = &line.commit {
                spans.push(Span::styled(
                    commit.sha.chars().take(8).collect::<String>(),
                    Style::default().fg(Color::Magenta),
                ));
                spans.push(Span::raw(" "));
                if !commit.refs.is_empty() {
                    spans.push(Span::styled(
                        format!("({}) ", commit.refs),
                        Style::default().fg(Color::Cyan),
                    ));
                }
                spans.push(Span::raw(commit.subject.clone()));
                spans.push(Span::styled(
                    format!("  {} {}", commit.author, time::age(&commit.date)),
                    Style::default().add_modifier(Modifier::DIM),
                ));
                let y = area.y + (index - top) as u16;
                hits.push((Rect::new(area.x, y, area.width, 1), Action::Select(index)));
            }
            let style = if index == self.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            drawn.push(Line::from(spans).style(style));
        }
        frame.render_widget(Paragraph::new(drawn), area);
        hits
    }

    /// The text of the current tab (or of the open commit).
    pub fn lines(&self) -> Vec<Line<'static>> {
        if let Some((sha, shown)) = &self.commit {
            return match shown {
                Some(text) => crate::diff::lines(text),
                None => vec![Line::raw(format!("Loading {sha}…"))],
            };
        }
        let changes = match &self.changes {
            Some(Ok(changes)) => changes,
            Some(Err(error)) => return vec![Line::raw(error.clone())],
            None => return vec![Line::raw("Loading…")],
        };
        let (files, diff, empty) = match self.tab {
            LocalTab::Staged => (&changes.status.staged, &changes.staged, "Nothing staged"),
            _ => (
                &changes.status.unstaged,
                &changes.working,
                "No unstaged changes",
            ),
        };
        let status = &changes.status;
        let mut branch = format!("Branch {}", status.branch);
        if let Some(upstream) = &status.upstream {
            branch.push_str(&format!(
                " → {upstream} · ↑{} ↓{}",
                status.ahead, status.behind
            ));
        }
        let mut lines = vec![
            Line::styled(branch, Style::default().add_modifier(Modifier::BOLD)),
            Line::raw(""),
        ];
        if files.is_empty() {
            lines.push(Line::raw(empty));
            return lines;
        }
        for (index, (path, code)) in files.iter().enumerate() {
            let line = Line::raw(format!("  {code} {path}"));
            lines.push(if index == self.file {
                line.style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                line
            });
        }
        lines.push(Line::raw(""));
        // Only the selected file's diff, cut from the whole one already read.
        if let Some((path, code)) = files.get(self.file) {
            let one = crate::git::diff_of(diff, path);
            if one.is_empty() {
                let what = if *code == '?' {
                    "New file"
                } else {
                    "No text diff for"
                };
                lines.push(Line::raw(format!("{what} {path}")));
            }
            let mut diff = crate::diff::lines(&one);
            if let Some(line) = self.line.and_then(|line| diff.get_mut(line)) {
                line.style = line.style.add_modifier(Modifier::REVERSED);
            }
            lines.extend(diff);
        }
        lines
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::git::Commit;

    pub(crate) fn history() -> Vec<GraphLine> {
        let commit = |sha: &str, subject: &str| {
            Some(Commit {
                sha: sha.repeat(40),
                author: "T".into(),
                date: "2026-10-27T10:00:00Z".into(),
                subject: subject.into(),
                refs: String::new(),
            })
        };
        vec![
            GraphLine {
                graph: "* ".into(),
                commit: commit("a", "Second"),
            },
            GraphLine {
                graph: "|\\ ".into(),
                commit: None,
            },
            GraphLine {
                graph: "* ".into(),
                commit: commit("b", "First"),
            },
        ]
    }

    #[test]
    fn selection_skips_graph_only_lines() {
        let mut local = Local::new(Err("none".into()));
        local.history = Some(Ok(history()));
        local.move_selection(1);
        assert_eq!(local.selected, 2);
        assert_eq!(local.selected_sha(), Some("b".repeat(40)));
        local.move_selection(-5);
        assert_eq!(local.selected, 0);
    }

    #[test]
    fn working_and_staged_show_branch_files_and_diff() {
        let mut local = Local::new(Err("none".into()));
        local.changes = Some(Ok(Changes {
            status: Status {
                branch: "main".into(),
                upstream: Some("origin/main".into()),
                ahead: 1,
                behind: 0,
                staged: vec![("a.rs".into(), 'M')],
                unstaged: vec![],
            },
            working: String::new(),
            staged: "diff --git a/a.rs b/a.rs\n+new".into(),
            author: "T <t@x>".into(),
        }));
        local.tab = LocalTab::Working;
        let text: Vec<String> = local.lines().iter().map(|l| l.to_string()).collect();
        assert_eq!(text[0], "Branch main → origin/main · ↑1 ↓0");
        assert!(text.contains(&"No unstaged changes".to_string()));
        local.tab = LocalTab::Staged;
        let text: Vec<String> = local.lines().iter().map(|l| l.to_string()).collect();
        assert!(text.contains(&"  M a.rs".to_string()) && text.contains(&"+new".to_string()));
    }
}
