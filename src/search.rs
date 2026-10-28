//! The Search view: a GitHub search query and the issues and pull requests it finds.

use crate::app::Action;
use crate::github::search::Hit;
use crate::time;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table},
};

pub struct SearchView {
    pub query: String,
    /// Keys type into the query.
    pub editing: bool,
    /// The results and GitHub's total count, for the query in `ran`.
    pub results: Option<Result<(Vec<Hit>, i64), String>>,
    /// The query the results (or the running read) belong to.
    pub ran: Option<String>,
    pub loading: bool,
    pub selected: usize,
}

impl Default for SearchView {
    fn default() -> Self {
        Self {
            query: String::new(),
            editing: true,
            results: None,
            ran: None,
            loading: false,
            selected: 0,
        }
    }
}

impl SearchView {
    fn count(&self) -> usize {
        match &self.results {
            Some(Ok((hits, _))) => hits.len(),
            _ => 0,
        }
    }

    pub fn selected_hit(&self) -> Option<&Hit> {
        match &self.results {
            Some(Ok((hits, _))) => hits.get(self.selected),
            _ => None,
        }
    }

    pub fn select(&mut self, index: usize) {
        self.selected = index.min(self.count().saturating_sub(1));
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.select(self.selected.saturating_add_signed(delta));
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let [input, results] =
            Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(area);
        let style = if self.editing {
            Style::default().fg(crate::theme::p().yellow)
        } else {
            Style::default()
        };
        let cursor = if self.editing { "▏" } else { "" };
        let block = crate::theme::pane(" Search ").border_style(style);
        frame.render_widget(
            Paragraph::new(format!("{}{cursor}", self.query)).block(block),
            input,
        );
        let mut hits = vec![(input, Action::SearchEdit)];
        let title = match (&self.results, &self.ran) {
            (Some(Ok((found, total))), Some(query)) => {
                format!(" {query} · {} of {total} ", found.len())
            }
            _ => " Results ".into(),
        };
        let block = crate::theme::pane(title);
        let inner = block.inner(results);
        frame.render_widget(block, results);
        let found = match &self.results {
            _ if self.loading => return message(frame, inner, &["Searching…"], hits),
            None => {
                let help = [
                    "Type GitHub search syntax and press Enter, for example:",
                    "  is:pr is:open review-requested:@me",
                    "  repo:owner/name is:issue label:bug",
                    "  involves:@me updated:>2026-10-01",
                ];
                return message(frame, inner, &help, hits);
            }
            Some(Err(error)) => return message(frame, inner, &[error.as_str()], hits),
            Some(Ok((found, _))) if found.is_empty() => {
                return message(frame, inner, &["Nothing matches"], hits);
            }
            Some(Ok((found, _))) => found,
        };
        let shown = usize::from(inner.height.saturating_sub(1));
        let top = self.selected.saturating_sub(shown.saturating_sub(1));
        let rows = found
            .iter()
            .enumerate()
            .skip(top)
            .take(shown)
            .map(|(index, hit)| {
                let style = if index == self.selected && !self.editing {
                    crate::theme::selected()
                } else {
                    Style::default()
                };
                let colour = match hit.state.as_str() {
                    "open" => crate::theme::p().green,
                    "merged" => crate::theme::p().accent,
                    "closed" => crate::theme::p().red,
                    _ => crate::theme::p().overlay,
                };
                let kind = if hit.pr { "PR" } else { "Issue" };
                Row::new(vec![
                    Cell::from(Span::styled(hit.state.clone(), Style::default().fg(colour))),
                    Cell::from(kind),
                    Cell::from(hit.reference()),
                    Cell::from(hit.title.clone()),
                    Cell::from(hit.author.clone()),
                    Cell::from(time::age(&hit.updated_at)),
                ])
                .style(style)
            });
        let table = Table::new(
            rows.collect::<Vec<_>>(),
            [
                Constraint::Length(7),
                Constraint::Length(5),
                Constraint::Length(32),
                Constraint::Fill(1),
                Constraint::Length(16),
                Constraint::Length(4),
            ],
        )
        .header(
            Row::new(["STATE", "TYPE", "REFERENCE", "TITLE", "AUTHOR", "AGE"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        );
        frame.render_widget(table, inner);
        hits.extend(
            (0..found.len().saturating_sub(top).min(shown)).map(|offset| {
                let rect = Rect::new(inner.x, inner.y + 1 + offset as u16, inner.width, 1);
                (rect, Action::Select(top + offset))
            }),
        );
        hits
    }
}

fn message(
    frame: &mut Frame,
    area: Rect,
    text: &[&str],
    hits: Vec<(Rect, Action)>,
) -> Vec<(Rect, Action)> {
    let lines: Vec<Line> = text
        .iter()
        .map(|line| Line::raw(line.to_string()))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
    hits
}
