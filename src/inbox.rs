//! The Inbox view: PRs in tabs, a table and a preview of the selected PR.

use crate::app::Action;
use crate::model::{PullRequest, Queue};
use crate::time;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
};

/// Width from which the preview sits beside the list.
const SPLIT_WIDTH: u16 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    All,
    Authored,
    Assigned,
    Review,
    Merged,
}

impl Tab {
    pub const ALL: [(Tab, char, &'static str); 5] = [
        (Tab::All, 'a', "All"),
        (Tab::Authored, 'c', "Created by me"),
        (Tab::Assigned, 'm', "Assigned to me"),
        (Tab::Review, 'v', "Needs my review"),
        (Tab::Merged, 'M', "Merged"),
    ];

    fn accepts(self, pr: &PullRequest) -> bool {
        match self {
            Tab::All | Tab::Merged => true,
            Tab::Authored => pr.queues.contains(&Queue::Authored),
            Tab::Assigned => pr.queues.contains(&Queue::Assigned),
            Tab::Review => pr.queues.contains(&Queue::Review),
        }
    }
}

/// A list that has been loaded, is loading, or failed.
#[derive(Default)]
pub struct Loaded {
    pub prs: Option<Vec<PullRequest>>,
    pub loading: bool,
    pub error: Option<String>,
    pub fetched_at: Option<i64>,
}

impl Loaded {
    pub fn finish(&mut self, result: Result<Vec<PullRequest>, String>) {
        self.loading = false;
        match result {
            Ok(prs) => {
                self.prs = Some(prs);
                self.error = None;
                self.fetched_at = Some(time::now());
            }
            Err(error) => self.error = Some(error),
        }
    }
}

pub struct Inbox {
    pub open: Loaded,
    pub merged: Loaded,
    pub tab: Tab,
    pub selected: usize,
    /// Repositories (`owner/name`) never shown.
    pub hidden: Vec<String>,
}

impl Default for Inbox {
    fn default() -> Self {
        Self {
            open: Loaded::default(),
            merged: Loaded::default(),
            tab: Tab::All,
            selected: 0,
            hidden: Vec::new(),
        }
    }
}

impl Inbox {
    fn source(&self) -> &Loaded {
        if self.tab == Tab::Merged {
            &self.merged
        } else {
            &self.open
        }
    }

    /// The PRs the current tab shows, in order.
    pub fn visible(&self) -> Vec<&PullRequest> {
        self.source()
            .prs
            .iter()
            .flatten()
            .filter(|pr| self.tab.accepts(pr) && !self.hidden.iter().any(|h| h == &pr.repo))
            .collect()
    }

    pub fn selected_pr(&self) -> Option<&PullRequest> {
        self.visible().get(self.selected).copied()
    }

    pub fn select(&mut self, index: usize) {
        let count = self.visible().len();
        self.selected = index.min(count.saturating_sub(1));
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.select(self.selected.saturating_add_signed(delta));
    }

    pub fn count(&self, tab: Tab) -> usize {
        let source = if tab == Tab::Merged {
            &self.merged
        } else {
            &self.open
        };
        source
            .prs
            .iter()
            .flatten()
            .filter(|pr| tab.accepts(pr) && !self.hidden.iter().any(|h| h == &pr.repo))
            .count()
    }

    /// Draws the Inbox in `area`; returns its click targets.
    pub fn draw(&self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let mut hits = Vec::new();
        let [tabs, summary, content] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .areas(area);
        hits.extend(self.draw_tabs(frame, tabs));
        frame.render_widget(Paragraph::new(self.summary()), summary);
        let (list, preview) = if area.width >= SPLIT_WIDTH {
            let [list, preview] =
                Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
                    .areas(content);
            (list, Some(preview))
        } else {
            (content, None)
        };
        hits.extend(self.draw_list(frame, list));
        if let Some(preview) = preview {
            self.draw_preview(frame, preview);
        }
        hits
    }

    fn draw_tabs(&self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let mut spans = Vec::new();
        let mut hits = Vec::new();
        let mut x = area.x + 1;
        for (tab, key, label) in Tab::ALL {
            let text = format!("{key} {label}");
            let width = text.chars().count() as u16;
            if x + width > area.right() {
                break;
            }
            let style = if tab == self.tab {
                Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default()
            };
            spans.push(Span::styled(text, style));
            spans.push(Span::raw("   "));
            hits.push((Rect::new(x, area.y, width, 1), Action::Tab(tab)));
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

    fn summary(&self) -> String {
        let source = self.source();
        let counts = Tab::ALL[..4]
            .iter()
            .map(|(tab, _, label)| format!("{label} {}", self.count(*tab)))
            .collect::<Vec<_>>()
            .join(" · ");
        let state = if source.loading {
            "loading…".to_string()
        } else if let Some(error) = &source.error {
            error.clone()
        } else if let Some(at) = source.fetched_at {
            format!("fetched {} ago", time::age_at_seconds(at))
        } else {
            "not loaded".into()
        };
        if self.tab == Tab::Merged {
            let days = crate::github::inbox::MERGED_DAYS;
            format!(
                " Merged in the last {days} days: {} • {state}",
                self.count(Tab::Merged)
            )
        } else {
            format!(" {counts} • {state}")
        }
    }

    fn draw_list(&self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Pull requests ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let visible = self.visible();
        if visible.is_empty() {
            let text = if self.source().prs.is_none() {
                "Loading…"
            } else {
                "No pull requests here. Press r to refresh."
            };
            frame.render_widget(Paragraph::new(text), inner);
            return Vec::new();
        }
        // Keep the selection on screen: rows below the header.
        let rows_shown = inner.height.saturating_sub(1) as usize;
        let top = self.selected.saturating_sub(rows_shown.saturating_sub(1));
        let rows: Vec<Row> = visible
            .iter()
            .enumerate()
            .skip(top)
            .take(rows_shown)
            .map(|(index, pr)| {
                let style = if index == self.selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                let title = if pr.draft {
                    format!("{} [draft]", pr.title)
                } else {
                    pr.title.clone()
                };
                Row::new(vec![
                    Cell::from(pr.reference()),
                    Cell::from(title),
                    Cell::from(pr.author.login.clone()),
                    Cell::from(time::age(&pr.created_at)),
                    Cell::from(pr.status()),
                ])
                .style(style)
            })
            .collect();
        // The PR column fits the longest `owner/name#N` shown, within reason.
        let reference = visible
            .iter()
            .map(|pr| pr.reference().chars().count() as u16)
            .max()
            .unwrap_or(2)
            .clamp(10, 40);
        let table = Table::new(
            rows,
            [
                Constraint::Length(reference),
                Constraint::Fill(1),
                Constraint::Length(14),
                Constraint::Length(4),
                Constraint::Length(34),
            ],
        )
        .header(
            Row::new(["PR", "TITLE", "AUTHOR", "AGE", "STATUS"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        );
        frame.render_widget(table, inner);
        (0..visible.len().saturating_sub(top).min(rows_shown))
            .map(|offset| {
                let rect = Rect::new(inner.x, inner.y + 1 + offset as u16, inner.width, 1);
                (rect, Action::Select(top + offset))
            })
            .collect()
    }

    fn draw_preview(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default().borders(Borders::ALL).title(" Preview ");
        let Some(pr) = self.selected_pr() else {
            frame.render_widget(
                Paragraph::new("No pull request selected").block(block),
                area,
            );
            return;
        };
        frame.render_widget(
            Paragraph::new(preview_lines(pr))
                .wrap(Wrap { trim: false })
                .block(block),
            area,
        );
    }
}

/// The preview of one PR: summary, reviewers and description.
pub fn preview_lines(pr: &PullRequest) -> Vec<Line<'static>> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut lines = vec![
        Line::styled(pr.title.clone(), bold),
        Line::raw(format!("{}  {}", pr.reference(), pr.status())),
        Line::raw(""),
        Line::raw(format!(
            "Author   {}{}",
            pr.author.login,
            bot(pr.author.bot)
        )),
        Line::raw(format!(
            "Opened   {} ({} ago)",
            pr.created_at,
            time::age(&pr.created_at)
        )),
        Line::raw(format!("Updated  {}", pr.updated_at)),
        Line::raw(format!("Link     {}", pr.url)),
        Line::raw(""),
        Line::styled("REVIEWERS", bold),
    ];
    let reviewers: Vec<_> = pr
        .reviewers
        .iter()
        .filter(|r| !r.person.login.eq_ignore_ascii_case(&pr.author.login))
        .collect();
    if reviewers.is_empty() {
        lines.push(Line::raw("No reviewers or comments yet"));
    }
    for reviewer in reviewers {
        lines.push(Line::raw(format!(
            "{}{} · {}",
            reviewer.person.login,
            bot(reviewer.person.bot),
            reviewer.stance.label()
        )));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("DESCRIPTION", bold));
    if pr.body.trim().is_empty() {
        lines.push(Line::raw("No description"));
    }
    lines.extend(pr.body.lines().map(|line| Line::raw(line.to_owned())));
    lines
}

fn bot(is_bot: bool) -> &'static str {
    if is_bot { " [bot]" } else { "" }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Checks, Decision, Person, Reviewer, Stance};

    pub(crate) fn pr(number: i64, repo: &str, queues: &[Queue]) -> PullRequest {
        PullRequest {
            repo: repo.into(),
            number,
            title: format!("Change {number}"),
            url: format!("https://github.com/{repo}/pull/{number}"),
            body: "Body".into(),
            draft: false,
            author: Person {
                login: "alice".into(),
                bot: false,
            },
            created_at: "2026-10-27T10:00:00Z".into(),
            updated_at: "2026-10-27T11:00:00Z".into(),
            merged_at: None,
            queues: queues.to_vec(),
            decision: Decision::None,
            checks: Checks::Passing,
            reviewers: vec![Reviewer {
                person: Person {
                    login: "coderabbitai".into(),
                    bot: true,
                },
                stance: Stance::Commented,
            }],
        }
    }

    #[test]
    fn tabs_filter_queues_and_hidden_repositories() {
        let mut inbox = Inbox::default();
        inbox.open.prs = Some(vec![
            pr(1, "o/a", &[Queue::Authored]),
            pr(2, "o/b", &[Queue::Review]),
            pr(3, "o/hidden", &[Queue::Authored]),
        ]);
        inbox.hidden = vec!["o/hidden".into()];
        assert_eq!(inbox.visible().len(), 2);
        inbox.tab = Tab::Review;
        assert_eq!(inbox.visible()[0].number, 2);
        assert_eq!(inbox.count(Tab::Authored), 1);
    }

    #[test]
    fn selection_stays_inside_the_list() {
        let mut inbox = Inbox::default();
        inbox.open.prs = Some(vec![pr(1, "o/a", &[]), pr(2, "o/a", &[])]);
        inbox.move_selection(5);
        assert_eq!(inbox.selected, 1);
        inbox.move_selection(-9);
        assert_eq!(inbox.selected, 0);
    }

    #[test]
    fn preview_lists_bots_and_leaves_out_the_author() {
        let text: Vec<String> = preview_lines(&pr(1, "o/a", &[]))
            .iter()
            .map(|line| line.to_string())
            .collect();
        assert!(
            text.contains(&"coderabbitai [bot] · commented".to_string()),
            "{text:?}"
        );
    }
}
