//! A small multi-line text editor: a buffer and a cursor, edited by keys and paste.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Editor {
    text: Vec<char>,
    /// Index into `text` where the next character goes.
    cursor: usize,
}

impl Editor {
    pub fn text(&self) -> String {
        self.text.iter().collect()
    }

    pub fn insert(&mut self, text: &str) {
        for c in text.chars().filter(|c| *c == '\n' || !c.is_control()) {
            self.text.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    /// Applies an editing key; returns whether it was one.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        match key.code {
            KeyCode::Char(c) => self.insert(&c.to_string()),
            KeyCode::Enter => self.insert("\n"),
            KeyCode::Tab => self.insert("    "),
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.text.remove(self.cursor);
            }
            KeyCode::Delete if self.cursor < self.text.len() => {
                self.text.remove(self.cursor);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.text.len()),
            KeyCode::Home => self.cursor = self.line_start(),
            KeyCode::End => {
                while self.cursor < self.text.len() && self.text[self.cursor] != '\n' {
                    self.cursor += 1;
                }
            }
            KeyCode::Backspace | KeyCode::Delete => {}
            _ => return false,
        }
        true
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor]
            .iter()
            .rposition(|c| *c == '\n')
            .map_or(0, |newline| newline + 1)
    }

    /// The text as lines, with `▏` at the cursor.
    pub fn lines_with_cursor(&self) -> Vec<String> {
        let mut text: String = self.text[..self.cursor].iter().collect();
        text.push('▏');
        text.extend(&self.text[self.cursor..]);
        text.split('\n').map(str::to_owned).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_keys(editor: &mut Editor, keys: &[KeyCode]) {
        for code in keys {
            editor.key(KeyEvent::new(*code, KeyModifiers::NONE));
        }
    }

    #[test]
    fn typing_editing_and_moving() {
        let mut editor = Editor::default();
        editor.insert("helo");
        type_keys(
            &mut editor,
            &[KeyCode::Left, KeyCode::Char('l'), KeyCode::End],
        );
        type_keys(
            &mut editor,
            &[KeyCode::Enter, KeyCode::Char('x'), KeyCode::Backspace],
        );
        editor.insert("world");
        assert_eq!(editor.text(), "hello\nworld");
        type_keys(&mut editor, &[KeyCode::Home]);
        assert_eq!(editor.lines_with_cursor(), ["hello", "▏world"]);
    }

    #[test]
    fn paste_keeps_newlines_and_drops_other_controls() {
        let mut editor = Editor::default();
        editor.insert("a\u{1b}[2J\nb");
        assert_eq!(editor.text(), "a[2J\nb");
        assert!(!editor.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)));
    }
}
