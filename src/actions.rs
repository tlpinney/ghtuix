//! The Actions view: the repository's workflow runs, and one run's jobs and steps.

use crate::app::Action;
use crate::github::actions::{Job, Run};
use crate::model::Checks;
use crate::time;
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

/// One run opened from the list.
pub struct OpenRun {
    pub run: Run,
    pub jobs: Option<Result<Vec<Job>, String>>,
    /// Index into `jobs`.
    pub selected: usize,
}

pub struct ActionsView {
    /// `owner/name` of the checkout's GitHub repository, or why there is none.
    pub repo: Result<String, String>,
    pub runs: Option<Result<Vec<Run>, String>>,
    pub loading: bool,
    pub selected: usize,
    pub open: Option<OpenRun>,
}

impl ActionsView {
    pub fn new(repo: Result<String, String>) -> Self {
        Self {
            repo,
            runs: None,
            loading: false,
            selected: 0,
            open: None,
        }
    }

    fn run_count(&self) -> usize {
        self.runs
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map_or(0, Vec::len)
    }

    pub fn selected_run(&self) -> Option<&Run> {
        self.runs.as_ref()?.as_ref().ok()?.get(self.selected)
    }

    pub fn selected_job(&self) -> Option<&Job> {
        let open = self.open.as_ref()?;
        open.jobs.as_ref()?.as_ref().ok()?.get(open.selected)
    }

    /// Selects row `index` of whichever list shows.
    pub fn select(&mut self, index: usize) {
        match &mut self.open {
            Some(open) => {
                let count = open
                    .jobs
                    .as_ref()
                    .and_then(|j| j.as_ref().ok())
                    .map_or(0, Vec::len);
                open.selected = index.min(count.saturating_sub(1));
            }
            None => self.selected = index.min(self.run_count().saturating_sub(1)),
        }
    }

    /// The selected row of whichever list shows.
    pub fn current(&self) -> usize {
        self.open
            .as_ref()
            .map_or(self.selected, |open| open.selected)
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.select(self.current().saturating_add_signed(delta));
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let repo = match &self.repo {
            Ok(repo) => repo.clone(),
            Err(error) => {
                let block = Block::default().borders(Borders::ALL).title(" Actions ");
                frame.render_widget(Paragraph::new(error.clone()).block(block), area);
                return Vec::new();
            }
        };
        match &self.open {
            Some(open) => draw_run(frame, area, &repo, open),
            None => self.draw_runs(frame, area, &repo),
        }
    }

    fn draw_runs(&self, frame: &mut Frame, area: Rect, repo: &str) -> Vec<(Rect, Action)> {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {repo} · runs "));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let runs = match &self.runs {
            None => return message(frame, inner, "Loading runs…"),
            Some(Err(error)) => return message(frame, inner, error),
            Some(Ok(runs)) if runs.is_empty() => {
                return message(frame, inner, "No workflow runs yet");
            }
            Some(Ok(runs)) => runs,
        };
        let shown = usize::from(inner.height.saturating_sub(1));
        let top = self.selected.saturating_sub(shown.saturating_sub(1));
        let rows = runs
            .iter()
            .enumerate()
            .skip(top)
            .take(shown)
            .map(|(index, run)| {
                let style = if index == self.selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                Row::new(vec![
                    mark(run.state()),
                    Cell::from(run.name.clone().unwrap_or_default()),
                    Cell::from(format!("#{} {}", run.run_number, run.display_title)),
                    Cell::from(run.head_branch.clone().unwrap_or_default()),
                    Cell::from(run.event.clone()),
                    Cell::from(
                        run.actor
                            .as_ref()
                            .map_or(String::new(), |a| a.login.clone()),
                    ),
                    Cell::from(time::age(&run.created_at)),
                ])
                .style(style)
            });
        let table = Table::new(
            rows.collect::<Vec<_>>(),
            [
                Constraint::Length(2),
                Constraint::Length(18),
                Constraint::Fill(1),
                Constraint::Length(18),
                Constraint::Length(12),
                Constraint::Length(14),
                Constraint::Length(4),
            ],
        )
        .header(
            Row::new(["", "WORKFLOW", "RUN", "BRANCH", "EVENT", "ACTOR", "AGE"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        );
        frame.render_widget(table, inner);
        row_hits(inner, top, runs.len().saturating_sub(top).min(shown), 1)
    }
}

fn draw_run(frame: &mut Frame, area: Rect, repo: &str, open: &OpenRun) -> Vec<(Rect, Action)> {
    let run = &open.run;
    let title = format!(
        " {repo} · {} #{} · {} ",
        run.name.clone().unwrap_or_default(),
        run.run_number,
        run.outcome()
    );
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let jobs = match &open.jobs {
        None => return message(frame, inner, "Loading jobs…"),
        Some(Err(error)) => return message(frame, inner, error),
        Some(Ok(jobs)) => jobs,
    };
    // One line per job, its steps below the selected one.
    let mut lines = Vec::new();
    let mut hits = Vec::new();
    for (index, job) in jobs.iter().enumerate() {
        let style = if index == open.selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        let row = lines.len() as u16;
        if row < inner.height {
            let rect = Rect::new(inner.x, inner.y + row, inner.width, 1);
            hits.push((rect, Action::Select(index)));
        }
        let outcome = job
            .conclusion
            .as_deref()
            .unwrap_or(&job.status)
            .replace('_', " ");
        lines.push(
            Line::from(vec![
                symbol(job.state()),
                Span::raw(format!("{}  ", job.name)),
                Span::styled(outcome, Style::default().add_modifier(Modifier::DIM)),
            ])
            .style(style),
        );
        if index == open.selected {
            for step in &job.steps {
                let state =
                    crate::github::actions::state(Some(&step.status), step.conclusion.as_deref());
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    symbol(state),
                    Span::raw(format!("{}. {}", step.number, step.name)),
                ]));
            }
        }
    }
    if jobs.is_empty() {
        lines.push(Line::raw("No jobs"));
    }
    frame.render_widget(Paragraph::new(lines), inner);
    hits
}

fn message(frame: &mut Frame, area: Rect, text: &str) -> Vec<(Rect, Action)> {
    frame.render_widget(Paragraph::new(text.to_owned()), area);
    Vec::new()
}

/// Click targets for `count` table rows starting at `top`, under a header of `header` rows.
fn row_hits(area: Rect, top: usize, count: usize, header: u16) -> Vec<(Rect, Action)> {
    (0..count)
        .map(|offset| {
            let rect = Rect::new(area.x, area.y + header + offset as u16, area.width, 1);
            (rect, Action::Select(top + offset))
        })
        .collect()
}

fn colour(state: Checks) -> (&'static str, Color) {
    match state {
        Checks::Passing => ("✓", Color::Green),
        Checks::Failing => ("✗", Color::Red),
        Checks::Pending | Checks::None => ("•", Color::Yellow),
    }
}

fn mark(state: Checks) -> Cell<'static> {
    let (text, color) = colour(state);
    Cell::from(text).style(Style::default().fg(color))
}

fn symbol(state: Checks) -> Span<'static> {
    let (text, color) = colour(state);
    Span::styled(format!("{text} "), Style::default().fg(color))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::github::actions::Step;

    pub(crate) fn run(id: i64, conclusion: Option<&str>) -> Run {
        Run {
            id,
            name: Some("CI".into()),
            display_title: format!("Change {id}"),
            status: Some(
                if conclusion.is_some() {
                    "completed"
                } else {
                    "in_progress"
                }
                .into(),
            ),
            conclusion: conclusion.map(str::to_owned),
            head_branch: Some("main".into()),
            event: "push".into(),
            run_number: id,
            created_at: "2026-10-27T10:00:00Z".into(),
            html_url: format!("https://github.com/o/a/actions/runs/{id}"),
            actor: None,
            run_started_at: Some("2026-10-27T10:00:00Z".into()),
            updated_at: Some("2026-10-27T10:05:00Z".into()),
        }
    }

    pub(crate) fn job(id: i64, conclusion: &str) -> Job {
        Job {
            id,
            name: format!("job {id}"),
            status: "completed".into(),
            conclusion: Some(conclusion.into()),
            started_at: None,
            html_url: Some(format!("https://github.com/o/a/actions/runs/1/job/{id}")),
            steps: vec![Step {
                number: 1,
                name: "cargo test".into(),
                status: "completed".into(),
                conclusion: Some(conclusion.into()),
            }],
        }
    }

    #[test]
    fn selection_follows_the_list_that_shows() {
        let mut view = ActionsView::new(Ok("o/a".into()));
        view.runs = Some(Ok(vec![run(1, Some("success")), run(2, Some("failure"))]));
        view.move_selection(5);
        assert_eq!(view.selected_run().unwrap().id, 2);
        view.open = Some(OpenRun {
            run: run(2, Some("failure")),
            jobs: Some(Ok(vec![job(10, "success"), job(11, "failure")])),
            selected: 0,
        });
        view.move_selection(1);
        assert_eq!(view.selected_job().unwrap().id, 11);
        assert_eq!(view.selected, 1, "the run list keeps its place");
    }
}
