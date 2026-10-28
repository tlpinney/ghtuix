//! Application state and the one dispatch path for keys, clicks and background results.

use crate::keybar::{self, Control};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

/// The top-level views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Inbox,
    Local,
}

impl View {
    const ALL: [(View, char, &'static str); 2] =
        [(View::Inbox, '1', "Inbox"), (View::Local, '2', "Local")];
}

/// Everything a key, a click or a background result can ask the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Show(View),
    Refresh,
    Quit,
}

/// Results that background work sends back to the UI thread.
#[derive(Debug)]
pub enum Message {
    SignedIn(Result<String, String>),
}

/// What the UI thread asks the background to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    SignIn,
}

pub struct App {
    pub view: View,
    pub login: Option<String>,
    pub status: String,
    pub quit: bool,
    /// Jobs requested since the event loop last looked.
    pub jobs: Vec<Job>,
    /// Click targets from the last frame.
    hits: Vec<(Rect, Action)>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            view: View::Inbox,
            login: None,
            status: "Signing in…".into(),
            quit: false,
            jobs: vec![Job::SignIn],
            hits: Vec::new(),
        }
    }
}

impl App {
    /// The single entry point for every action.
    pub fn dispatch(&mut self, action: Action) {
        match action {
            Action::Show(view) => self.view = view,
            Action::Refresh => {
                self.status = "Refreshing…".into();
                self.jobs.push(Job::SignIn);
            }
            Action::Quit => self.quit = true,
        }
    }

    pub fn receive(&mut self, message: Message) {
        match message {
            Message::SignedIn(Ok(login)) => {
                self.status = format!("Signed in as @{login}");
                self.login = Some(login);
            }
            Message::SignedIn(Err(error)) => self.status = error,
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        if let Some(action) = keybar::resolve(&self.controls(), key) {
            self.dispatch(action);
        }
    }

    pub fn mouse(&mut self, mouse: MouseEvent) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let at = Position::new(mouse.column, mouse.row);
        if let Some((_, action)) = self.hits.iter().rev().find(|(rect, _)| rect.contains(at)) {
            self.dispatch(action.clone());
        }
    }

    /// The controls of the current view, in priority order (the key bar draws what fits).
    fn controls(&self) -> Vec<Control> {
        let mut controls: Vec<Control> = View::ALL
            .iter()
            .map(|(view, key, label)| Control::new(KeyCode::Char(*key), label, Action::Show(*view)))
            .collect();
        controls.push(Control::new(KeyCode::Char('r'), "Refresh", Action::Refresh));
        controls.push(Control::new(KeyCode::Char('q'), "Quit", Action::Quit));
        controls
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let [header, body, status, bar] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        let account = self
            .login
            .as_deref()
            .map_or(String::new(), |login| format!("@{login}"));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" ghtuix ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(account),
            ])),
            header,
        );
        let title = View::ALL
            .iter()
            .find(|(view, ..)| *view == self.view)
            .unwrap()
            .2;
        frame.render_widget(Block::default().borders(Borders::ALL).title(title), body);
        frame.render_widget(Paragraph::new(format!(" {}", self.status)), status);
        self.hits = keybar::draw(frame, bar, &self.controls());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};

    fn press(app: &mut App, c: char) {
        app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }

    #[test]
    fn view_keys_and_clicks_switch_views() {
        let mut app = App::default();
        press(&mut app, '2');
        assert_eq!(app.view, View::Local);
        let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let (rect, _) = app
            .hits
            .iter()
            .find(|(_, a)| *a == Action::Show(View::Inbox))
            .unwrap()
            .clone();
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.view, View::Inbox);
    }

    #[test]
    fn refresh_queues_work_and_results_update_status() {
        let mut app = App::default();
        app.jobs.clear();
        press(&mut app, 'r');
        assert_eq!(app.jobs, vec![Job::SignIn]);
        app.receive(Message::SignedIn(Ok("octocat".into())));
        assert_eq!(app.login.as_deref(), Some("octocat"));
        press(&mut app, 'q');
        assert!(app.quit);
    }
}
