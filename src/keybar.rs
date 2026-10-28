//! The key bar: every view's controls as `key Label` entries. Drawing it records each
//! entry's area so a click dispatches the same `Action` as the key.

use crate::app::Action;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// One control: the key that triggers it, its label and its action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Control {
    pub key: KeyCode,
    pub label: &'static str,
    pub action: Action,
}

impl Control {
    pub fn new(key: KeyCode, label: &'static str, action: Action) -> Self {
        Self { key, label, action }
    }

    fn key_text(&self) -> String {
        match self.key {
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "Enter".into(),
            KeyCode::Esc => "Esc".into(),
            KeyCode::Left => "←".into(),
            KeyCode::Tab => "Tab".into(),
            KeyCode::F(n) => format!("F{n}"),
            other => format!("{other:?}"),
        }
    }

    fn width(&self) -> u16 {
        (self.key_text().chars().count() + 1 + self.label.chars().count()) as u16
    }
}

/// The action bound to `key`, if any control has it. Shift is implied by an uppercase char.
pub fn resolve(controls: &[Control], key: KeyEvent) -> Option<Action> {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    controls
        .iter()
        .find(|control| control.key == key.code)
        .map(|control| control.action.clone())
}

/// Draws the controls left to right in `area`; returns each drawn control's click target.
/// Controls that do not fit are left off (they stay reachable by key).
pub fn draw(frame: &mut Frame, area: Rect, controls: &[Control]) -> Vec<(Rect, Action)> {
    let mut spans = Vec::new();
    let mut hits = Vec::new();
    let mut x = area.x + 1;
    for control in controls {
        let width = control.width();
        if x + width > area.right() {
            break;
        }
        spans.push(Span::styled(
            control.key_text(),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(format!(" {}  ", control.label)));
        hits.push((Rect::new(x, area.y, width, 1), control.action.clone()));
        x += width + 2;
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn controls() -> Vec<Control> {
        vec![
            Control::new(KeyCode::Char('r'), "Refresh", Action::Refresh),
            Control::new(KeyCode::Char('q'), "Quit", Action::Quit),
        ]
    }

    #[test]
    fn keys_and_clicks_carry_the_same_action() {
        let mut terminal = Terminal::new(TestBackend::new(40, 1)).unwrap();
        let mut hits = Vec::new();
        terminal
            .draw(|frame| hits = draw(frame, frame.area(), &controls()))
            .unwrap();
        let line: String = (0..40)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol().to_owned())
            .collect();
        assert!(line.contains("r Refresh  q Quit"), "{line}");
        for control in controls() {
            let hit = hits
                .iter()
                .find(|(_, action)| *action == control.action)
                .unwrap();
            let key = KeyEvent::new(control.key, KeyModifiers::NONE);
            assert_eq!(resolve(&controls(), key), Some(hit.1.clone()));
        }
    }

    #[test]
    fn controls_that_do_not_fit_are_left_off() {
        let mut terminal = Terminal::new(TestBackend::new(12, 1)).unwrap();
        let mut hits = Vec::new();
        terminal
            .draw(|frame| hits = draw(frame, frame.area(), &controls()))
            .unwrap();
        assert_eq!(hits.len(), 1);
    }
}
