//! Application state and the one dispatch path for keys, clicks and background results.

use crate::inbox::{Inbox, Tab};
use crate::keybar::{self, Control};
use crate::model::PullRequest;
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

/// Everything a key or a click can ask the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Show(View),
    Tab(Tab),
    Select(usize),
    Move(isize),
    OpenInBrowser,
    Refresh,
    Quit,
}

/// Results that background work sends back to the UI thread.
#[derive(Debug)]
pub enum Message {
    SignedIn(Result<String, String>),
    Inbox(Result<Vec<PullRequest>, String>),
    Merged(Result<Vec<PullRequest>, String>),
}

/// What the UI thread asks the background to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    SignIn,
    LoadInbox,
    LoadMerged,
    Open(String),
}

pub struct App {
    pub view: View,
    pub inbox: Inbox,
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
            inbox: Inbox::default(),
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
            Action::Tab(tab) => {
                self.inbox.tab = tab;
                self.inbox.select(0);
                if tab == Tab::Merged && self.inbox.merged.prs.is_none() {
                    self.load(Job::LoadMerged);
                }
            }
            Action::Select(index) => self.inbox.select(index),
            Action::Move(delta) => self.inbox.move_selection(delta),
            Action::OpenInBrowser => match self.inbox.selected_pr() {
                Some(pr) => {
                    self.status = format!("Opening {} in the browser", pr.reference());
                    self.jobs.push(Job::Open(pr.url.clone()));
                }
                None => self.status = "Select a pull request first".into(),
            },
            Action::Refresh => {
                if self.login.is_none() {
                    self.jobs.push(Job::SignIn);
                } else if self.inbox.tab == Tab::Merged {
                    self.load(Job::LoadMerged);
                } else {
                    self.load(Job::LoadInbox);
                }
            }
            Action::Quit => self.quit = true,
        }
    }

    /// Queues a list load unless one is already running.
    fn load(&mut self, job: Job) {
        let list = match job {
            Job::LoadMerged => &mut self.inbox.merged,
            _ => &mut self.inbox.open,
        };
        if !list.loading {
            list.loading = true;
            self.jobs.push(job);
        }
    }

    pub fn receive(&mut self, message: Message) {
        match message {
            Message::SignedIn(Ok(login)) => {
                self.status = format!("Signed in as @{login}");
                self.login = Some(login);
                self.load(Job::LoadInbox);
            }
            Message::SignedIn(Err(error)) => self.status = error,
            Message::Inbox(result) => self.inbox.open.finish(result),
            Message::Merged(result) => self.inbox.merged.finish(result),
        }
        self.inbox.move_selection(0);
    }

    pub fn key(&mut self, key: KeyEvent) {
        let action = keybar::resolve(&self.controls(), key).or(match key.code {
            KeyCode::Down | KeyCode::Char('j') => Some(Action::Move(1)),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::Move(-1)),
            KeyCode::PageDown => Some(Action::Move(10)),
            KeyCode::PageUp => Some(Action::Move(-10)),
            _ => None,
        });
        if let Some(action) = action {
            self.dispatch(action);
        }
    }

    pub fn mouse(&mut self, mouse: MouseEvent) {
        let action = match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let at = Position::new(mouse.column, mouse.row);
                self.hits
                    .iter()
                    .rev()
                    .find(|(rect, _)| rect.contains(at))
                    .map(|(_, action)| action.clone())
            }
            MouseEventKind::ScrollDown => Some(Action::Move(3)),
            MouseEventKind::ScrollUp => Some(Action::Move(-3)),
            _ => None,
        };
        if let Some(action) = action {
            self.dispatch(action);
        }
    }

    /// The controls of the current view, in priority order (the key bar draws what fits).
    fn controls(&self) -> Vec<Control> {
        let mut controls = Vec::new();
        if self.view == View::Inbox {
            controls.push(Control::new(
                KeyCode::Char('w'),
                "Web",
                Action::OpenInBrowser,
            ));
            controls.extend(
                Tab::ALL
                    .iter()
                    .map(|(tab, key, _)| Control::hidden(KeyCode::Char(*key), Action::Tab(*tab))),
            );
        }
        controls.push(Control::new(KeyCode::Char('r'), "Refresh", Action::Refresh));
        controls.extend(View::ALL.iter().map(|(view, key, label)| {
            Control::new(KeyCode::Char(*key), label, Action::Show(*view))
        }));
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
        let mut hits = match self.view {
            View::Inbox => self.inbox.draw(frame, body),
            View::Local => {
                frame.render_widget(
                    Paragraph::new(" Local Git comes in a later step.")
                        .block(Block::default().borders(Borders::ALL).title(" Local ")),
                    body,
                );
                Vec::new()
            }
        };
        frame.render_widget(Paragraph::new(format!(" {}", self.status)), status);
        hits.extend(keybar::draw(frame, bar, &self.controls()));
        self.hits = hits;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::tests::pr;
    use crate::model::Queue;
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};

    fn press(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn draw(app: &mut App, width: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..20)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect()
            })
            .collect()
    }

    fn click(app: &mut App, action: &Action) {
        let (rect, _) = app.hits.iter().find(|(_, a)| a == action).unwrap().clone();
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        });
    }

    fn signed_in() -> App {
        let mut app = App::default();
        app.receive(Message::SignedIn(Ok("octocat".into())));
        app.receive(Message::Inbox(Ok(vec![
            pr(1, "o/a", &[Queue::Authored]),
            pr(2, "o/b", &[Queue::Review]),
        ])));
        app.jobs.clear();
        app
    }

    #[test]
    fn signing_in_loads_the_inbox_once() {
        let mut app = App::default();
        app.jobs.clear();
        app.receive(Message::SignedIn(Ok("octocat".into())));
        assert_eq!(app.jobs, vec![Job::LoadInbox]);
        app.dispatch(Action::Refresh);
        assert_eq!(
            app.jobs,
            vec![Job::LoadInbox],
            "no second load while one runs"
        );
    }

    #[test]
    fn tabs_rows_and_web_work_by_key_and_by_click() {
        for by_click in [false, true] {
            let mut app = signed_in();
            let screen = draw(&mut app, 140).join("\n");
            assert!(
                screen.contains("o/a#1") && screen.contains("REVIEWERS"),
                "{screen}"
            );
            if by_click {
                click(&mut app, &Action::Tab(Tab::Review));
            } else {
                press(&mut app, KeyCode::Char('v'));
            }
            assert_eq!(app.inbox.selected_pr().unwrap().number, 2);
            draw(&mut app, 140);
            if by_click {
                click(&mut app, &Action::OpenInBrowser);
            } else {
                press(&mut app, KeyCode::Char('w'));
            }
            assert_eq!(
                app.jobs,
                vec![Job::Open("https://github.com/o/b/pull/2".into())]
            );
        }
    }

    #[test]
    fn merged_tab_loads_on_demand_and_rows_select_by_click() {
        let mut app = signed_in();
        press(&mut app, KeyCode::Char('M'));
        assert_eq!(app.jobs, vec![Job::LoadMerged]);
        app.jobs.clear();
        press(&mut app, KeyCode::Char('a'));
        draw(&mut app, 140);
        click(&mut app, &Action::Select(1));
        assert_eq!(app.inbox.selected, 1);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.inbox.selected, 0);
        press(&mut app, KeyCode::Char('q'));
        assert!(app.quit);
    }
}
