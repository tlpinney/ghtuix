mod app;
mod github;
mod keybar;

use anyhow::Result;
use app::{App, Job, Message};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

const HOST: &str = "github.com";

fn main() -> Result<()> {
    let mut terminal = ratatui::init();
    crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
    let result = run(&mut terminal);
    crossterm::execute!(std::io::stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
    let mut app = App::default();
    let (tx, rx) = mpsc::channel();
    while !app.quit {
        for job in app.jobs.drain(..) {
            spawn(job, tx.clone());
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
fn spawn(job: Job, tx: Sender<Message>) {
    std::thread::spawn(move || {
        let message = match job {
            Job::SignIn => Message::SignedIn(
                github::Client::connect(HOST)
                    .and_then(|client| github::viewer::login(&client))
                    .map_err(|error| format!("{error:#}")),
            ),
        };
        let _ = tx.send(message);
    });
}
