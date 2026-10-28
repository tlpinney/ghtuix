//! The Notifications view: GitHub's notification threads for the signed-in account.

use crate::app::Action;
use crate::github::notifications::Notification;
use crate::time;
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

#[derive(Default)]
pub struct NotificationsView {
    pub items: Option<Result<Vec<Notification>, String>>,
    /// Read notifications too, not only unread ones.
    pub all: bool,
    pub loading: bool,
    pub selected: usize,
}

impl NotificationsView {
    fn count(&self) -> usize {
        self.items
            .as_ref()
            .and_then(|items| items.as_ref().ok())
            .map_or(0, Vec::len)
    }

    pub fn selected_item(&self) -> Option<&Notification> {
        self.items.as_ref()?.as_ref().ok()?.get(self.selected)
    }

    pub fn select(&mut self, index: usize) {
        self.selected = index.min(self.count().saturating_sub(1));
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.select(self.selected.saturating_add_signed(delta));
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let which = if self.all { "all" } else { "unread" };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Notifications · {which} "));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let items = match &self.items {
            None => return message(frame, inner, "Loading notifications…"),
            Some(Err(error)) => return message(frame, inner, error),
            Some(Ok(items)) if items.is_empty() => {
                let text = if self.all {
                    "No notifications"
                } else {
                    "No unread notifications (a shows all)"
                };
                return message(frame, inner, text);
            }
            Some(Ok(items)) => items,
        };
        let shown = usize::from(inner.height.saturating_sub(1));
        let top = self.selected.saturating_sub(shown.saturating_sub(1));
        let rows = items
            .iter()
            .enumerate()
            .skip(top)
            .take(shown)
            .map(|(index, item)| {
                let mut style = Style::default();
                if item.unread {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if index == self.selected {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                let mark = if item.unread { "●" } else { " " };
                Row::new(vec![
                    Cell::from(mark).style(Style::default().fg(Color::Blue)),
                    Cell::from(item.repository.full_name.clone()),
                    Cell::from(kind(&item.subject.kind)),
                    Cell::from(item.subject.title.clone()),
                    Cell::from(item.reason.replace('_', " ")),
                    Cell::from(time::age(&item.updated_at)),
                ])
                .style(style)
            });
        let table = Table::new(
            rows.collect::<Vec<_>>(),
            [
                Constraint::Length(1),
                Constraint::Length(28),
                Constraint::Length(7),
                Constraint::Fill(1),
                Constraint::Length(16),
                Constraint::Length(4),
            ],
        )
        .header(
            Row::new(["", "REPOSITORY", "TYPE", "TITLE", "REASON", "AGE"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        );
        frame.render_widget(table, inner);
        (0..items.len().saturating_sub(top).min(shown))
            .map(|offset| {
                let rect = Rect::new(inner.x, inner.y + 1 + offset as u16, inner.width, 1);
                (rect, Action::Select(top + offset))
            })
            .collect()
    }
}

/// A short name for the subject's kind.
fn kind(kind: &str) -> &str {
    match kind {
        "PullRequest" => "PR",
        "CheckSuite" => "CI",
        "Discussion" => "Discuss",
        "RepositoryVulnerabilityAlert" | "RepositoryDependabotAlertsThread" => "Alert",
        other => other,
    }
}

fn message(frame: &mut Frame, area: Rect, text: &str) -> Vec<(Rect, Action)> {
    frame.render_widget(Paragraph::new(text.to_owned()), area);
    Vec::new()
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::github::notifications::{Notification, Repository, Subject};

    pub(crate) fn notification(id: &str, kind: &str, number: i64) -> Notification {
        Notification {
            id: id.into(),
            unread: true,
            reason: "review_requested".into(),
            updated_at: "2026-10-27T10:00:00Z".into(),
            subject: Subject {
                title: format!("Subject {id}"),
                url: Some(format!(
                    "https://api.github.com/repos/o/a/{}/{number}",
                    if kind == "PullRequest" {
                        "pulls"
                    } else {
                        "issues"
                    }
                )),
                kind: kind.into(),
            },
            repository: Repository {
                full_name: "o/a".into(),
                html_url: "https://github.com/o/a".into(),
            },
        }
    }
}
