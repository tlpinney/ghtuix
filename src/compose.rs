//! The compose box for a write: edit, preview the exact effect, confirm.

use crate::app::Action;
use crate::editor::Editor;
use crate::model::{MergeMethod, PrPage, WriteKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
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
    pub base_ref: String,
    pub merge_state: String,
    pub merge_methods: Vec<MergeMethod>,
}

impl Target {
    /// A workflow run as the target of an Actions write (no PR fields).
    pub fn run(repo: &str, title: &str) -> Self {
        Self {
            reference: repo.to_owned(),
            title: title.to_owned(),
            pr_id: String::new(),
            head_oid: String::new(),
            base_ref: String::new(),
            merge_state: String::new(),
            merge_methods: Vec::new(),
        }
    }

    pub fn of(page: &PrPage) -> Self {
        Self {
            reference: format!("{}#{}", page.repo, page.number),
            title: page.title.clone(),
            pr_id: page.id.clone(),
            head_oid: page.head_oid.clone(),
            base_ref: page.base_ref.clone(),
            merge_state: page.merge_state.clone(),
            merge_methods: page.merge_methods.clone(),
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
            WriteKind::Merge { .. } => "Merge".into(),
            WriteKind::Rerun { run, repo, .. } => {
                format!("Re-run the failed jobs of {run} in {repo}")
            }
            WriteKind::CancelRun { run, repo, .. } => format!("Cancel {run} in {repo}"),
            WriteKind::Commit { branch, files, .. } => {
                format!("Commit {} staged file(s) on {branch} in", files.len())
            }
            WriteKind::Local { op } => {
                format!("{} in", WriteKind::Local { op: op.clone() }.label())
            }
            WriteKind::MarkRead { .. } => "Mark read the notification in".into(),
            WriteKind::MarkDone { .. } => "Mark done (leaves the inbox) the notification in".into(),
            WriteKind::Close => "Close".into(),
            WriteKind::Reopen => "Reopen".into(),
            WriteKind::ToDraft => "Convert to a draft".into(),
            WriteKind::Ready => "Mark ready for review".into(),
            WriteKind::Title { .. } => "Rename".into(),
            WriteKind::Labels { .. } => "Change the labels of".into(),
            WriteKind::Reviewers { .. } => "Change the review requests of".into(),
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
        if let WriteKind::Local { op } = &self.kind {
            use crate::git::LocalOp;
            // A local operation: the exact git command, and what it will do.
            lines[2] = Line::raw(format!("Runs: {}", op.command(&self.editor.text())));
            let note = match op {
                LocalOp::Switch(_) => {
                    "Uncommitted changes come along; git refuses if they would be overwritten."
                }
                LocalOp::CreateBranch => "The new branch starts at the current commit.",
                LocalOp::DeleteBranch(_) => "Only a fully merged branch; git refuses otherwise.",
                LocalOp::Rebase { .. } => "",
                LocalOp::RebaseContinue => "Uses the commit messages as they are.",
                LocalOp::RebaseAbort => "Returns the branch to where it was before the rebase.",
                LocalOp::StashPush => "Tracked and untracked changes are saved and removed.",
                LocalOp::StashApply(_) => "The stash stays in the list.",
                LocalOp::StashPop(_) => "The stash is removed once it applies cleanly.",
                LocalOp::StashDrop(_) => "The stash is deleted.",
            };
            if !note.is_empty() {
                lines.push(Line::raw(note));
            }
            if let LocalOp::Rebase { onto, plan } = op {
                lines.push(Line::raw(format!(
                    "Replays {} commit(s) onto {onto}, which has {} commit(s) this branch lacks:",
                    plan.replay.len(),
                    plan.incoming
                )));
                lines.extend(plan.replay.iter().map(|c| Line::raw(format!("  {c}"))));
                lines.push(Line::raw(
                    "If it stops on a conflict, Branches offers Continue and Abort.",
                ));
            }
        } else if let WriteKind::Commit { author, files, .. } = &self.kind {
            // A local commit: git's own author identity, not the GitHub account.
            lines[2] = Line::raw(format!("As {author}"));
            for file in files {
                lines.push(Line::raw(format!("  {file}")));
            }
        } else if let WriteKind::Merge { method } = &self.kind {
            lines[0] = Line::styled(
                format!(
                    "Merge {} into {} by {}",
                    self.target.reference,
                    self.target.base_ref,
                    method.label().to_lowercase()
                ),
                Style::default().add_modifier(Modifier::BOLD),
            );
            lines.push(Line::raw(format!(
                "GitHub merge state: {}",
                self.target.merge_state
            )));
            lines.push(Line::raw(format!(
                "Only if the head is still {short}; GitHub refuses if anyone pushed since."
            )));
        } else if matches!(
            self.kind,
            WriteKind::Approve | WriteKind::RequestChanges | WriteKind::LineComment { .. }
        ) {
            lines.push(Line::raw(format!("On head commit {short}")));
        } else if let WriteKind::Title { from } = &self.kind {
            let to = crate::github::write::new_title(&self.editor.text());
            lines.push(Line::raw(format!("From: {from}")));
            lines.push(Line::raw(format!("To:   {to}")));
        } else if let WriteKind::Labels { add, remove } = &self.kind {
            lines.extend(add.iter().map(|l| Line::raw(format!("  + {}", l.name))));
            lines.extend(remove.iter().map(|l| Line::raw(format!("  − {}", l.name))));
        } else if let WriteKind::Reviewers { add, remove, .. } = &self.kind {
            lines.extend(add.iter().map(|login| Line::raw(format!("  + @{login}"))));
            lines.extend(
                remove
                    .iter()
                    .map(|login| Line::raw(format!("  − @{login}"))),
            );
        }
        if self.kind.has_text() && !matches!(self.kind, WriteKind::Title { .. }) {
            lines.push(Line::raw(""));
            let text = self.editor.text();
            if text.trim().is_empty() {
                lines.push(Line::raw("(no text)"));
            }
            lines.extend(text.lines().map(|line| Line::raw(format!("│ {line}"))));
        }
        lines.push(Line::raw(""));
        let local = matches!(
            self.kind,
            WriteKind::Commit { .. } | WriteKind::Local { .. }
        );
        lines.push(Line::raw(if local {
            "Nothing runs until you confirm."
        } else {
            "Nothing is sent until you confirm."
        }));
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
        let block = crate::theme::pane(title);
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
        // A merge offers the repository's allowed methods before its confirm button.
        let methods: Vec<(String, Action)> = match &self.kind {
            WriteKind::Merge { method } if self.stage == Stage::Preview => self
                .target
                .merge_methods
                .iter()
                .map(|candidate| {
                    let (_, key, label) = MergeMethod::ALL
                        .into_iter()
                        .find(|(m, ..)| m == candidate)
                        .unwrap();
                    let mark = if candidate == method { "●" } else { "○" };
                    (
                        format!("{mark} {key} {label}"),
                        Action::MergeMethod(*candidate),
                    )
                })
                .collect(),
            _ => Vec::new(),
        };
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
        let owned: Vec<(String, Action)> = methods
            .into_iter()
            .chain(
                choices
                    .iter()
                    .map(|(label, action)| (label.to_string(), action.clone())),
            )
            .collect();
        for (label, action) in &owned {
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
