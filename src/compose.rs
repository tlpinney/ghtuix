//! The compose box for a write: edit, preview the exact effect, confirm.

use crate::app::Action;
use crate::editor::Editor;
use crate::model::{PrPage, WriteKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Editing,
    Preview,
    Sending,
}

/// The PR a write goes to, as the user saw it when composing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub reference: String,
    pub title: String,
    pub pr_id: String,
    pub head_oid: String,
}

impl Target {
    pub fn of(page: &PrPage) -> Self {
        Self {
            reference: format!("{}#{}", page.repo, page.number),
            title: page.title.clone(),
            pr_id: page.id.clone(),
            head_oid: page.head_oid.clone(),
        }
    }
}

pub struct Compose {
    pub kind: WriteKind,
    pub target: Target,
    pub editor: Editor,
    pub stage: Stage,
    pub error: Option<String>,
}

impl Compose {
    pub fn new(kind: WriteKind, target: Target) -> Self {
        // A write without text (resolving a thread) starts at its preview.
        let stage = if kind.has_text() {
            Stage::Editing
        } else {
            Stage::Preview
        };
        Self {
            kind,
            target,
            editor: Editor::default(),
            stage,
            error: None,
        }
    }

    /// Moves to the preview, or says why it cannot.
    pub fn preview(&mut self) {
        if !self.kind.body_optional() && self.editor.text().trim().is_empty() {
            self.error = Some(format!("{} needs some text", self.kind.label()));
            return;
        }
        self.error = None;
        self.stage = Stage::Preview;
    }

    /// The preview: exactly what Confirm sends, where and as whom.
    pub fn preview_lines(&self, login: &str) -> Vec<Line<'static>> {
        let effect = match &self.kind {
            WriteKind::Comment => "Add a comment to the conversation of".to_string(),
            WriteKind::Approve => "Approve".into(),
            WriteKind::RequestChanges => "Request changes on".into(),
            WriteKind::LineComment { path, line, side } => {
                let side = if *side == crate::model::Side::Left {
                    "old"
                } else {
                    "new"
                };
                format!("Comment on {path} line {line} ({side} side) of")
            }
            WriteKind::Reply { place, .. } => format!("Reply in the thread on {place} of"),
            WriteKind::Resolve { place, resolve, .. } => {
                let verb = if *resolve { "Resolve" } else { "Unresolve" };
                format!("{verb} the review thread on {place} of")
            }
        };
        let short = self.target.head_oid.chars().take(12).collect::<String>();
        let mut lines = vec![
            Line::styled(
                format!("{effect} {}", self.target.reference),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::raw(self.target.title.clone()),
            Line::raw(format!("As @{login}")),
        ];
        if matches!(
            self.kind,
            WriteKind::Approve | WriteKind::RequestChanges | WriteKind::LineComment { .. }
        ) {
            lines.push(Line::raw(format!("On head commit {short}")));
        }
        if self.kind.has_text() {
            lines.push(Line::raw(""));
            let text = self.editor.text();
            if text.trim().is_empty() {
                lines.push(Line::raw("(no text)"));
            }
            lines.extend(text.lines().map(|line| Line::raw(format!("│ {line}"))));
        }
        lines.push(Line::raw(""));
        lines.push(Line::raw("Nothing is sent until you confirm."));
        lines
    }

    /// Draws the box over `area`; returns its buttons' click targets.
    pub fn draw(&self, frame: &mut Frame, area: Rect, login: &str) -> Vec<(Rect, Action)> {
        let width = area.width.saturating_sub(8).clamp(20, 90);
        let height = area.height.saturating_sub(4).clamp(8, 24);
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        let rect = Rect::new(x, y, width, height);
        let title = format!(" {} • {} ", self.kind.label(), self.target.reference);
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(rect);
        frame.render_widget(Clear, rect);
        frame.render_widget(block, rect);
        let [body, error, buttons] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(inner);
        let lines = match self.stage {
            Stage::Editing => self
                .editor
                .lines_with_cursor()
                .into_iter()
                .map(Line::raw)
                .collect(),
            Stage::Preview | Stage::Sending => self.preview_lines(login),
        };
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
        if let Some(message) = &self.error {
            frame.render_widget(Paragraph::new(message.clone()), error);
        }
        let choices: &[(&str, Action)] = match self.stage {
            Stage::Editing => &[
                ("Ctrl-S Preview", Action::ComposePreview),
                ("Esc Cancel", Action::ComposeCancel),
            ],
            Stage::Preview if self.kind.has_text() => &[
                ("Enter Confirm", Action::ComposeConfirm),
                ("Esc Edit", Action::ComposeEdit),
            ],
            Stage::Preview => &[
                ("Enter Confirm", Action::ComposeConfirm),
                ("Esc Cancel", Action::ComposeCancel),
            ],
            Stage::Sending => &[],
        };
        let mut spans = Vec::new();
        let mut hits = Vec::new();
        let mut at = buttons.x;
        if self.stage == Stage::Sending {
            spans.push(Span::raw("Sending…"));
        }
        for (label, action) in choices {
            let text = format!("[{label}]");
            let width = text.chars().count() as u16;
            hits.push((Rect::new(at, buttons.y, width, 1), action.clone()));
            spans.push(Span::styled(
                text,
                Style::default().add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw("  "));
            at += width + 2;
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), buttons);
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> Target {
        Target::of(&crate::pr::tests::page())
    }

    #[test]
    fn a_comment_needs_text_but_an_approval_does_not() {
        let mut comment = Compose::new(WriteKind::Comment, target());
        comment.preview();
        assert_eq!(comment.stage, Stage::Editing);
        assert!(comment.error.is_some());
        let mut approve = Compose::new(WriteKind::Approve, target());
        approve.preview();
        assert_eq!(approve.stage, Stage::Preview);
    }

    #[test]
    fn the_preview_states_the_exact_effect() {
        let mut compose = Compose::new(WriteKind::RequestChanges, target());
        compose.editor.insert("Please split this");
        compose.preview();
        let text: Vec<String> = compose
            .preview_lines("octocat")
            .iter()
            .map(|line| line.to_string())
            .collect();
        assert_eq!(text[0], "Request changes on o/a#1");
        assert!(text.contains(&"As @octocat".to_string()));
        assert!(text.contains(&format!("On head commit {}", "a".repeat(12))));
        assert!(text.contains(&"│ Please split this".to_string()));
    }
}
