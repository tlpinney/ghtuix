//! The Dashboard: PR and CI numbers for the checkout's repository over 7 or 30 days. Each
//! card opens the records behind its number; a partial or unknown value says so.

use crate::app::Action;
use crate::github::actions::Run;
use crate::github::dashboard::{Counted, Metrics, PrRecord};
use crate::model::Checks;
use crate::time;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
};

/// Where a record leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Pr {
        repo: String,
        number: i64,
        title: String,
        url: String,
    },
    Web(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub reference: String,
    pub title: String,
    pub who: String,
    /// The timestamp the card is about (opened, merged, run started).
    pub when: String,
    pub detail: String,
    pub target: Target,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub title: &'static str,
    pub value: String,
    pub note: String,
    pub records: Vec<Record>,
}

pub struct DashboardView {
    pub repo: Result<String, String>,
    pub days: i64,
    pub metrics: Option<Result<Metrics, String>>,
    pub loading: bool,
    pub selected: usize,
    /// The card whose records show, and the selected record.
    pub open: Option<usize>,
    pub row: usize,
}

impl DashboardView {
    pub fn new(repo: Result<String, String>) -> Self {
        Self {
            repo,
            days: 7,
            metrics: None,
            loading: false,
            selected: 0,
            open: None,
            row: 0,
        }
    }

    pub fn cards(&self) -> Vec<Card> {
        match &self.metrics {
            Some(Ok(metrics)) => cards(metrics, time::now()),
            _ => Vec::new(),
        }
    }

    pub fn selected_record(&self) -> Option<Record> {
        let card = self.cards().into_iter().nth(self.open?)?;
        card.records.into_iter().nth(self.row)
    }

    /// Selects card or record `index`, whichever shows.
    pub fn select(&mut self, index: usize) {
        let cards = self.cards();
        match self.open {
            Some(card) => {
                let count = cards.get(card).map_or(0, |c| c.records.len());
                self.row = index.min(count.saturating_sub(1));
            }
            None => self.selected = index.min(cards.len().saturating_sub(1)),
        }
    }

    pub fn current(&self) -> usize {
        if self.open.is_some() {
            self.row
        } else {
            self.selected
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.select(self.current().saturating_add_signed(delta));
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let repo = match &self.repo {
            Ok(repo) => repo,
            Err(error) => {
                let block = Block::default().borders(Borders::ALL).title(" Dashboard ");
                frame.render_widget(Paragraph::new(error.clone()).block(block), area);
                return Vec::new();
            }
        };
        let title = format!(" {repo} · last {} days ", self.days);
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let note = match &self.metrics {
            _ if self.loading && self.metrics.is_none() => Some("Loading…".to_string()),
            None => Some("Loading…".into()),
            Some(Err(error)) => Some(error.clone()),
            Some(Ok(_)) => None,
        };
        if let Some(note) = note {
            frame.render_widget(Paragraph::new(note), inner);
            return Vec::new();
        }
        let cards = self.cards();
        match self.open.and_then(|index| cards.get(index)) {
            Some(card) => draw_records(frame, inner, card, self.row),
            None => draw_cards(frame, inner, &cards, self.selected),
        }
    }
}

/// Three cards a row, five lines each.
fn draw_cards(
    frame: &mut Frame,
    area: Rect,
    cards: &[Card],
    selected: usize,
) -> Vec<(Rect, Action)> {
    let mut hits = Vec::new();
    let width = area.width / 3;
    for (index, card) in cards.iter().enumerate() {
        let (column, row) = ((index % 3) as u16, (index / 3) as u16);
        let rect = Rect::new(area.x + column * width, area.y + row * 5, width, 5);
        if rect.bottom() > area.bottom() {
            break;
        }
        let style = if index == selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", card.title))
            .title_style(style);
        let lines = vec![
            Line::styled(
                card.value.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::raw(card.note.clone()),
        ];
        frame.render_widget(
            Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
            rect,
        );
        hits.push((rect, Action::Select(index)));
    }
    hits
}

fn draw_records(frame: &mut Frame, area: Rect, card: &Card, row: usize) -> Vec<(Rect, Action)> {
    let [heading, table_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    frame.render_widget(
        Paragraph::new(format!("{}: {} · {}", card.title, card.value, card.note))
            .style(Style::default().add_modifier(Modifier::BOLD)),
        heading,
    );
    if card.records.is_empty() {
        frame.render_widget(Paragraph::new("No records"), table_area);
        return Vec::new();
    }
    let shown = usize::from(table_area.height.saturating_sub(1));
    let top = row.saturating_sub(shown.saturating_sub(1));
    let rows = card
        .records
        .iter()
        .enumerate()
        .skip(top)
        .take(shown)
        .map(|(index, record)| {
            let style = if index == row {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(record.reference.clone()),
                Cell::from(record.title.clone()),
                Cell::from(record.who.clone()),
                Cell::from(time::age(&record.when)),
                Cell::from(record.detail.clone()),
            ])
            .style(style)
        });
    let table = Table::new(
        rows.collect::<Vec<_>>(),
        [
            Constraint::Length(12),
            Constraint::Fill(1),
            Constraint::Length(16),
            Constraint::Length(4),
            Constraint::Length(22),
        ],
    )
    .header(
        Row::new(["RECORD", "TITLE", "WHO", "AGE", "DETAIL"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    );
    frame.render_widget(table, table_area);
    (0..card.records.len().saturating_sub(top).min(shown))
        .map(|offset| {
            let y = table_area.y + 1 + offset as u16;
            (
                Rect::new(table_area.x, y, table_area.width, 1),
                Action::Select(top + offset),
            )
        })
        .collect()
}

/// The middle value (the mean of the middle two for an even count), or `None`.
fn median(mut values: Vec<i64>) -> Option<i64> {
    values.sort_unstable();
    let middle = values.len() / 2;
    match values.len() {
        0 => None,
        n if n % 2 == 0 => Some((values[middle - 1] + values[middle]) / 2),
        _ => Some(values[middle]),
    }
}

/// `N`, or `at least N` when GitHub counted more records than it returned.
fn of<T>(counted: &Counted<T>, what: &str) -> String {
    if counted.partial() {
        format!(
            "from the newest {} of {} {what}",
            counted.records.len(),
            counted.total
        )
    } else {
        format!("from all {} {what}", counted.records.len())
    }
}

fn pr_record(repo: &str, pr: &PrRecord, when: &str, detail: String) -> Record {
    Record {
        reference: format!("#{}", pr.number),
        title: pr.title.clone(),
        who: pr.author.clone(),
        when: when.to_owned(),
        detail,
        target: Target::Pr {
            repo: repo.to_owned(),
            number: pr.number,
            title: pr.title.clone(),
            url: pr.url.clone(),
        },
    }
}

fn run_record(run: &Run, detail: String) -> Record {
    Record {
        reference: format!("run {}", run.run_number),
        title: format!(
            "{} · {}",
            run.name.clone().unwrap_or_default(),
            run.display_title
        ),
        who: run
            .actor
            .as_ref()
            .map_or(String::new(), |a| a.login.clone()),
        when: run.created_at.clone(),
        detail,
        target: Target::Web(run.html_url.clone()),
    }
}

/// How long a finished run took.
fn run_seconds(run: &Run) -> Option<i64> {
    if run.status.as_deref() != Some("completed") {
        return None;
    }
    let start = time::parse(run.run_started_at.as_deref()?)?;
    let end = time::parse(run.updated_at.as_deref()?)?;
    Some(end - start)
}

pub fn cards(metrics: &Metrics, now: i64) -> Vec<Card> {
    let repo = &metrics.repo;
    let total = |counted: i64, partial: bool| {
        if partial {
            format!("{counted} (list shows 100)")
        } else {
            counted.to_string()
        }
    };
    // Opened.
    let opened = Card {
        title: "PRs opened",
        value: total(metrics.opened.total, metrics.opened.partial()),
        note: format!("since {}", metrics.since),
        records: metrics
            .opened
            .records
            .iter()
            .map(|pr| {
                let state = if pr.merged_at.is_some() {
                    "merged"
                } else if pr.draft {
                    "draft"
                } else {
                    ""
                };
                pr_record(repo, pr, &pr.created_at, state.into())
            })
            .collect(),
    };
    // Merged, with the time each took.
    let took =
        |pr: &PrRecord| Some(time::parse(pr.merged_at.as_deref()?)? - time::parse(&pr.created_at)?);
    let merge_times: Vec<i64> = metrics.merged.records.iter().filter_map(took).collect();
    let merged = Card {
        title: "PRs merged",
        value: total(metrics.merged.total, metrics.merged.partial()),
        note: match median(merge_times) {
            Some(seconds) => format!(
                "median {} to merge, {}",
                time::span(seconds),
                of(&metrics.merged, "merged")
            ),
            None => "no merges to time".into(),
        },
        records: metrics
            .merged
            .records
            .iter()
            .map(|pr| {
                let detail = took(pr).map_or(String::new(), |s| format!("took {}", time::span(s)));
                pr_record(repo, pr, pr.merged_at.as_deref().unwrap_or(""), detail)
            })
            .collect(),
    };
    // Open now, and how many have waited longer than a week.
    let week = 7 * 86_400;
    let stale = metrics
        .open
        .records
        .iter()
        .filter(|pr| time::parse(&pr.created_at).is_some_and(|at| now - at > week))
        .count();
    let open = Card {
        title: "Open PRs",
        value: total(metrics.open.total, metrics.open.partial()),
        note: if metrics.open.partial() {
            format!("at least {stale} open over a week")
        } else {
            format!("{stale} open over a week")
        },
        records: metrics
            .open
            .records
            .iter()
            .map(|pr| {
                let detail = if pr.draft { "draft" } else { "" };
                pr_record(repo, pr, &pr.created_at, detail.into())
            })
            .collect(),
    };
    let waiting = Card {
        title: "Waiting for review",
        value: total(metrics.waiting.total, metrics.waiting.partial()),
        note: "open, not drafts, review required".into(),
        records: metrics
            .waiting
            .records
            .iter()
            .map(|pr| {
                let detail = format!("waiting {}", time::age_at(&pr.created_at, now));
                pr_record(repo, pr, &pr.created_at, detail)
            })
            .collect(),
    };
    // Workflow runs: success rate over finished runs, failures, run time.
    let runs = &metrics.runs;
    let finished: Vec<&Run> = runs
        .records
        .iter()
        .filter(|run| run.status.as_deref() == Some("completed"))
        .collect();
    let failed: Vec<&Run> = finished
        .iter()
        .copied()
        .filter(|run| run.state() == Checks::Failing)
        .collect();
    let rate = if finished.is_empty() {
        "no finished runs".to_string()
    } else {
        let passed = finished.len() - failed.len();
        format!(
            "{}% passed, {}",
            passed * 100 / finished.len(),
            of(runs, "runs")
        )
    };
    let run_card = Card {
        title: "Workflow runs",
        value: total(runs.total, runs.partial()),
        note: rate,
        records: runs
            .records
            .iter()
            .map(|run| run_record(run, run.outcome()))
            .collect(),
    };
    let failed_card = Card {
        title: "Failed runs",
        value: if runs.partial() {
            format!("at least {}", failed.len())
        } else {
            failed.len().to_string()
        },
        note: if finished.is_empty() {
            "no finished runs".into()
        } else {
            format!("of {} finished", finished.len())
        },
        records: failed
            .iter()
            .map(|run| run_record(run, run.outcome()))
            .collect(),
    };
    let mut timed: Vec<(&Run, i64)> = runs
        .records
        .iter()
        .filter_map(|run| Some((run, run_seconds(run)?)))
        .collect();
    let run_time = median(timed.iter().map(|(_, seconds)| *seconds).collect());
    timed.sort_by_key(|(_, seconds)| std::cmp::Reverse(*seconds));
    let time_card = Card {
        title: "Run time",
        value: run_time.map_or("unknown".into(), |s| format!("{} median", time::span(s))),
        note: if timed.is_empty() {
            "no finished runs to time".into()
        } else {
            format!("slowest first, {} timed", timed.len())
        },
        records: timed
            .iter()
            .map(|(run, seconds)| run_record(run, format!("took {}", time::span(*seconds))))
            .collect(),
    };
    vec![
        opened,
        merged,
        open,
        waiting,
        run_card,
        failed_card,
        time_card,
    ]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::actions::tests::run;

    pub(crate) fn pr(number: i64, created: &str, merged: Option<&str>) -> PrRecord {
        PrRecord {
            number,
            title: format!("PR {number}"),
            url: format!("https://github.com/o/a/pull/{number}"),
            author: "alice".into(),
            draft: false,
            created_at: created.into(),
            merged_at: merged.map(str::to_owned),
        }
    }

    pub(crate) fn metrics() -> Metrics {
        Metrics {
            repo: "o/a".into(),
            days: 7,
            since: "2026-10-20".into(),
            opened: Counted {
                records: vec![pr(1, "2026-10-26T00:00:00Z", None)],
                total: 1,
            },
            merged: Counted {
                records: vec![
                    pr(2, "2026-10-25T00:00:00Z", Some("2026-10-25T02:00:00Z")),
                    pr(3, "2026-10-25T00:00:00Z", Some("2026-10-26T00:00:00Z")),
                    pr(4, "2026-10-25T00:00:00Z", Some("2026-10-25T06:00:00Z")),
                ],
                total: 250,
            },
            open: Counted {
                records: vec![
                    pr(5, "2026-10-01T00:00:00Z", None),
                    pr(6, "2026-10-26T00:00:00Z", None),
                ],
                total: 2,
            },
            waiting: Counted {
                records: vec![pr(6, "2026-10-26T00:00:00Z", None)],
                total: 1,
            },
            runs: Counted {
                records: vec![
                    run(1, Some("success")),
                    run(2, Some("failure")),
                    run(3, None),
                ],
                total: 3,
            },
        }
    }

    #[test]
    fn cards_count_time_and_say_when_they_are_partial() {
        let now = time::parse("2026-10-27T00:00:00Z").unwrap();
        let cards = cards(&metrics(), now);
        let card = |title: &str| cards.iter().find(|c| c.title == title).unwrap().clone();
        let merged = card("PRs merged");
        assert_eq!(merged.value, "250 (list shows 100)");
        assert_eq!(
            merged.note,
            "median 6h to merge, from the newest 3 of 250 merged"
        );
        assert_eq!(merged.records[1].detail, "took 1d");
        assert_eq!(card("Open PRs").note, "1 open over a week");
        let runs = card("Workflow runs");
        assert_eq!(runs.note, "50% passed, from all 3 runs");
        let failed = card("Failed runs");
        assert_eq!((failed.value.as_str(), failed.records.len()), ("1", 1));
        assert_eq!(card("Run time").value, "5m median");
        assert_eq!(median(vec![600, 1260]), Some(930), "the middle two's mean");
    }

    #[test]
    fn nothing_to_measure_reads_unknown_not_zero() {
        let mut metrics = metrics();
        metrics.merged = Counted {
            records: Vec::new(),
            total: 0,
        };
        metrics.runs = Counted {
            records: Vec::new(),
            total: 0,
        };
        let cards = cards(&metrics, time::now());
        let card = |title: &str| cards.iter().find(|c| c.title == title).unwrap().clone();
        assert_eq!(card("PRs merged").note, "no merges to time");
        assert_eq!(card("Workflow runs").note, "no finished runs");
        assert_eq!(card("Run time").value, "unknown");
    }
}
