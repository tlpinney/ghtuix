//! A list to choose from, filtered by typing: one item (a menu) or several (labels,
//! reviewers). Choosing never writes anything; the app turns the choice into a preview.

use crate::app::Action;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

/// What the picker is choosing, so the app knows what its answer means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// The PR actions menu; an item's `id` names the action.
    PrMenu,
    Labels,
    Reviewers,
    Theme,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// What the app acts on: a menu action's name, a label's node id, a login.
    pub id: String,
    pub label: String,
    pub checked: bool,
}

impl Item {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            checked: false,
        }
    }
}

pub struct Picker {
    pub purpose: Purpose,
    pub title: String,
    /// Several items may be checked; otherwise choosing one is the answer.
    pub multi: bool,
    /// `None` while the items load, or why they could not.
    pub items: Option<Result<Vec<Item>, String>>,
    pub filter: String,
    /// Index into `shown()`.
    pub selected: usize,
    /// What Cancel puts back (the theme picker previews as it moves).
    pub restore: Option<String>,
}

impl Picker {
    pub fn new(purpose: Purpose, title: impl Into<String>, multi: bool) -> Self {
        Self {
            purpose,
            title: title.into(),
            multi,
            items: None,
            filter: String::new(),
            selected: 0,
            restore: None,
        }
    }

    /// Indexes into `items` of those matching the filter (case-insensitively).
    pub fn shown(&self) -> Vec<usize> {
        let needle = self.filter.to_lowercase();
        let Some(Ok(items)) = &self.items else {
            return Vec::new();
        };
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.label.to_lowercase().contains(&needle))
            .map(|(index, _)| index)
            .collect()
    }

    pub fn move_selection(&mut self, delta: isize) {
        let count = self.shown().len();
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(count.saturating_sub(1));
    }

    pub fn type_char(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.filter.pop();
        self.selected = 0;
    }

    /// The item under the cursor.
    pub fn current(&self) -> Option<&Item> {
        let index = *self.shown().get(self.selected)?;
        self.items.as_ref()?.as_ref().ok()?.get(index)
    }

    /// Checks or unchecks the item at row `row` of the filtered list.
    pub fn toggle(&mut self, row: usize) {
        let Some(&index) = self.shown().get(row) else {
            return;
        };
        self.selected = row;
        if let Some(Ok(items)) = &mut self.items {
            items[index].checked = !items[index].checked;
        }
    }

    /// Whether `id` is one of the items offered (an unlisted one is never changed).
    pub fn offers(&self, id: &str) -> bool {
        matches!(&self.items, Some(Ok(items)) if items.iter().any(|item| item.id == id))
    }

    pub fn checked(&self) -> Vec<&Item> {
        match &self.items {
            Some(Ok(items)) => items.iter().filter(|item| item.checked).collect(),
            _ => Vec::new(),
        }
    }

    /// Draws the box over `area`; returns its rows' and buttons' click targets.
    pub fn draw(&self, frame: &mut Frame, area: Rect) -> Vec<(Rect, Action)> {
        let width = area.width.saturating_sub(8).clamp(20, 60);
        let height = area.height.saturating_sub(4).clamp(8, 20);
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        let rect = Rect::new(x, y, width, height);
        let block = crate::theme::pane(format!(" {} ", self.title));
        let inner = block.inner(rect);
        frame.render_widget(Clear, rect);
        frame.render_widget(block, rect);
        let [filter, list, buttons] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(inner);
        frame.render_widget(Paragraph::new(format!("Filter: {}▏", self.filter)), filter);
        let mut hits = Vec::new();
        match &self.items {
            None => frame.render_widget(Paragraph::new("Loading…"), list),
            Some(Err(error)) => frame.render_widget(Paragraph::new(error.clone()), list),
            Some(Ok(items)) => {
                let shown = self.shown();
                let rows = usize::from(list.height);
                let top = self.selected.saturating_sub(rows.saturating_sub(1));
                let mut lines = Vec::new();
                for (row, &index) in shown.iter().enumerate().skip(top).take(rows) {
                    let item = &items[index];
                    let mark = match (self.multi, item.checked) {
                        (false, _) => "",
                        (true, true) => "[x] ",
                        (true, false) => "[ ] ",
                    };
                    let style = if row == self.selected {
                        crate::theme::selected()
                    } else {
                        Style::default()
                    };
                    let y = list.y + (row - top) as u16;
                    hits.push((Rect::new(list.x, y, list.width, 1), Action::PickerRow(row)));
                    lines.push(Line::styled(format!("{mark}{}", item.label), style));
                }
                if shown.is_empty() {
                    lines.push(Line::raw("Nothing matches"));
                }
                frame.render_widget(Paragraph::new(lines), list);
            }
        }
        let choices: &[(&str, Action)] = if self.multi {
            &[
                ("Space Toggle", Action::PickerToggle),
                ("Enter Done", Action::PickerDone),
                ("Esc Cancel", Action::PickerCancel),
            ]
        } else {
            &[
                ("Enter Choose", Action::PickerDone),
                ("Esc Cancel", Action::PickerCancel),
            ]
        };
        let mut spans = Vec::new();
        let mut at = buttons.x;
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

    fn picker() -> Picker {
        let mut picker = Picker::new(Purpose::Labels, "Labels", true);
        picker.items = Some(Ok(vec![
            Item::new("L1", "bug"),
            Item::new("L2", "docs"),
            Item::new("L3", "Big change"),
        ]));
        picker
    }

    #[test]
    fn typing_filters_and_toggling_acts_on_the_filtered_row() {
        let mut picker = picker();
        picker.type_char('b');
        assert_eq!(picker.shown(), vec![0, 2], "case-insensitive");
        picker.toggle(1);
        assert_eq!(picker.checked()[0].id, "L3");
        picker.backspace();
        assert_eq!(picker.shown().len(), 3);
        picker.move_selection(9);
        assert_eq!(picker.current().unwrap().id, "L3");
    }
}
