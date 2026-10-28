mod app;
mod config;
mod github;
mod inbox;
mod keybar;
mod model;
mod time;

use anyhow::Result;
use app::{App, Job, Message};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often the Inbox refreshes itself while ghtuix is open.
const REFRESH_EVERY: Duration = Duration::from_secs(60);

/// The signed-in client, shared by every background job once sign-in succeeds.
type Session = Arc<Mutex<Option<github::Client>>>;

fn main() -> Result<()> {
    let config = config::Config::load()?;
    let mut terminal = ratatui::init();
    crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
    let result = run(&mut terminal, config);
    crossterm::execute!(std::io::stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, config: config::Config) -> Result<()> {
    let host: Arc<str> = config.host().into();
    let mut app = App::default();
    app.inbox.hidden = config.excluded_repositories;
    let session = Session::default();
    let (tx, rx) = mpsc::channel();
    let mut last_refresh = Instant::now();
    while !app.quit {
        if last_refresh.elapsed() >= REFRESH_EVERY && app.login.is_some() {
            app.dispatch(app::Action::Refresh);
            last_refresh = Instant::now();
        }
        for job in app.jobs.drain(..) {
            spawn(job, host.clone(), session.clone(), tx.clone());
        }
        terminal.draw(|frame| app.draw(frame))?;
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.key(key),
                Event::Mouse(mouse) => app.mouse(mouse),
                _ => {}
            }
        }
        while let Ok(message) = rx.try_recv() {
            app.receive(message);
        }
    }
    Ok(())
}

/// Runs one job on a background thread; its result comes back as a `Message`.
fn spawn(job: Job, host: Arc<str>, session: Session, tx: Sender<Message>) {
    std::thread::spawn(move || {
        let client = || {
            session
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| "Not signed in yet".to_string())
        };
        let text = |error: anyhow::Error| format!("{error:#}");
        let message = match job {
            Job::SignIn => Message::SignedIn(
                github::Client::connect(&host)
                    .and_then(|client| {
                        let login = github::viewer::login(&client)?;
                        *session.lock().unwrap() = Some(client);
                        Ok(login)
                    })
                    .map_err(text),
            ),
            Job::LoadInbox => {
                Message::Inbox(client().and_then(|c| github::inbox::open(&c).map_err(text)))
            }
            Job::LoadMerged => {
                Message::Merged(client().and_then(|c| github::inbox::merged(&c).map_err(text)))
            }
            Job::Open(url) => {
                let opener = if cfg!(target_os = "macos") {
                    "open"
                } else {
                    "xdg-open"
                };
                let _ = std::process::Command::new(opener).arg(url).status();
                return;
            }
        };
        let _ = tx.send(message);
    });
}
