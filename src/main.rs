mod app;
mod cache;
mod compose;
mod config;
mod diff;
mod editor;
mod git;
mod github;
mod inbox;
mod keybar;
mod local;
mod log;
mod markdown;
mod model;
mod pr;
mod time;

use anyhow::Result;
use app::{App, Job, Message};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEventKind,
};
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
    crossterm::execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    let result = run(&mut terminal, config);
    crossterm::execute!(
        std::io::stdout(),
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, config: config::Config) -> Result<()> {
    let host: Arc<str> = config.host().into();
    let mut app = App::default();
    app.inbox.hidden = config.excluded_repositories;
    app.local = local::Local::new(
        std::env::current_dir()
            .map_err(anyhow::Error::from)
            .and_then(|dir| git::Repo::discover(&dir))
            .map_err(|_| "Not inside a Git checkout: start ghtuix in one to see it here".into()),
    );
    // A cache that cannot open only costs the instant start.
    let cache = cache::path().and_then(|path| cache::Cache::open(&path).ok());
    if let Some(cache) = &cache
        && let Some((login, _)) = cache.get::<String>(&cache::login_key(&host))
        && let Some((prs, saved_at)) = cache.get(&cache::inbox_key(&host, &login))
    {
        app.restore(login, prs, saved_at);
    }
    let session = Session::default();
    let (tx, rx) = mpsc::channel();
    let mut last_refresh = Instant::now();
    while !app.quit {
        // Automatic refresh follows the API budget: slower when it runs low, paused while
        // GitHub has asked to wait.
        let budget = session
            .lock()
            .unwrap()
            .as_ref()
            .map(|client| client.budget());
        let now = time::now();
        let interval = budget.map_or(REFRESH_EVERY, |b| b.refresh_interval(REFRESH_EVERY, now));
        app.api_note = budget.and_then(|b| b.note(now));
        if last_refresh.elapsed() >= interval && app.login.is_some() {
            app.dispatch(app::Action::Refresh);
            last_refresh = Instant::now();
        }
        let jobs: Vec<Job> = app.jobs.drain(..).collect();
        for job in jobs {
            // The cache answers what has not changed; only the rest goes to GitHub.
            match cache
                .as_ref()
                .and_then(|cache| from_cache(cache, &host, &job))
            {
                Some(message) => app.receive(message),
                None => spawn(job, host.clone(), session.clone(), tx.clone()),
            }
        }
        terminal.draw(|frame| app.draw(frame))?;
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.key(key),
                Event::Mouse(mouse) => app.mouse(mouse),
                Event::Paste(text) => app.paste(&text),
                _ => {}
            }
        }
        while let Ok(message) = rx.try_recv() {
            if let Some(cache) = &cache {
                save(cache, &host, &message);
            }
            app.receive(message);
        }
    }
    Ok(())
}

/// The cached answer to `job`, when the cache holds one that is still current: a PR page
/// with the same `updatedAt` as the Inbox row, or the diff between the same two commits.
fn from_cache(cache: &cache::Cache, host: &str, job: &Job) -> Option<Message> {
    let (login, _) = cache.get::<String>(&cache::login_key(host))?;
    match job {
        Job::LoadPr {
            repo,
            number,
            updated_at: Some(updated_at),
        } => {
            let key = cache::pr_key(host, &login, repo, *number);
            let (page, _) = cache.get::<model::PrPage>(&key)?;
            (page.updated_at == *updated_at).then(|| Message::Pr(Ok(Box::new(page))))
        }
        Job::LoadLog {
            repo,
            job_id,
            finished: true,
        } => {
            let (log, _) = cache.get::<String>(&cache::log_key(host, &login, repo, *job_id))?;
            Some(Message::Log {
                repo: repo.clone(),
                job_id: *job_id,
                finished: true,
                result: Ok(log),
            })
        }
        Job::LoadDiff { base, head, .. } => {
            let (diff, _) = cache.get::<String>(&cache::diff_key(host, &login, base, head))?;
            Some(Message::Diff {
                base: base.clone(),
                head: head.clone(),
                result: Ok(diff),
            })
        }
        _ => None,
    }
}

/// Keeps what a message brings for later. Cache errors are not worth an error.
fn save(cache: &cache::Cache, host: &str, message: &Message) {
    if let Message::SignedIn(Ok(login)) = message {
        let _ = cache.put(&cache::login_key(host), login);
        return;
    }
    let Some((login, _)) = cache.get::<String>(&cache::login_key(host)) else {
        return;
    };
    let _ = match message {
        Message::Inbox(Ok(prs)) => cache.put(&cache::inbox_key(host, &login), prs),
        Message::Pr(Ok(page)) => {
            cache.put(&cache::pr_key(host, &login, &page.repo, page.number), page)
        }
        Message::Diff {
            base,
            head,
            result: Ok(diff),
        } => cache.put(&cache::diff_key(host, &login, base, head), diff),
        // Only a finished job's log is final; a running job's log keeps growing.
        Message::Log {
            repo,
            job_id,
            finished: true,
            result: Ok(log),
        } => cache.put(&cache::log_key(host, &login, repo, *job_id), log),
        _ => Ok(()),
    };
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
            Job::LoadPr { repo, number, .. } => Message::Pr(client().and_then(|c| {
                github::pr::page(&c, &repo, number)
                    .map(Box::new)
                    .map_err(text)
            })),
            Job::LoadDiff {
                repo,
                number,
                base,
                head,
            } => Message::Diff {
                base,
                head,
                result: client().and_then(|c| github::pr::diff(&c, &repo, number).map_err(text)),
            },
            Job::LoadMerged => {
                Message::Merged(client().and_then(|c| github::inbox::merged(&c).map_err(text)))
            }
            Job::LoadLog {
                repo,
                job_id,
                finished,
            } => Message::Log {
                result: client().and_then(|c| github::pr::job_log(&c, &repo, job_id).map_err(text)),
                repo,
                job_id,
                finished,
            },
            Job::LoadHistory(repo) => Message::History(repo.history(500).map_err(text)),
            Job::LoadChanges(repo) => Message::Changes(
                repo.status()
                    .and_then(|status| {
                        Ok(local::Changes {
                            status,
                            working: repo.working_diff()?,
                            staged: repo.staged_diff()?,
                        })
                    })
                    .map_err(text),
            ),
            Job::ShowCommit(repo, sha) => Message::Commit {
                result: repo.show(&sha).map_err(text),
                sha,
            },
            Job::Write {
                kind,
                pr_id,
                head_oid,
                body,
            } => Message::Written(client().and_then(|c| {
                github::write::publish(&c, &kind, &pr_id, &head_oid, &body).map_err(text)
            })),
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
