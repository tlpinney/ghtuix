mod actions;
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
mod notifications;
mod picker;
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
    let checkout = std::env::current_dir()
        .map_err(anyhow::Error::from)
        .and_then(|dir| git::Repo::discover(&dir));
    app.actions = actions::ActionsView::new(match &checkout {
        Ok(repo) => repo.github_repo(&host).ok_or_else(|| {
            format!("This checkout has no origin remote on {host}, so it has no Actions here")
        }),
        Err(_) => Err("Not inside a Git checkout: start ghtuix in one to see its Actions".into()),
    });
    app.local = local::Local::new(
        checkout
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
            let job = match &cache {
                Some(cache) => with_saved(cache, &host, job),
                None => job,
            };
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

/// Where a conditional Actions read keeps its last body and `ETag`.
fn etag_what(repo: &str, run_id: Option<i64>) -> String {
    match run_id {
        Some(run_id) => format!("jobs/{repo}/{run_id}"),
        None => format!("runs/{repo}"),
    }
}

/// Where a conditional notifications read keeps its last body and `ETag`.
fn notifications_what(all: bool) -> String {
    format!("notifications/{}", if all { "all" } else { "unread" })
}

/// Adds the saved body and `ETag` to an Actions read, so GitHub can answer `304`.
fn with_saved(cache: &cache::Cache, host: &str, job: Job) -> Job {
    let Some((login, _)) = cache.get::<String>(&cache::login_key(host)) else {
        return job;
    };
    let saved = |what: String| {
        cache
            .get::<github::actions::Saved>(&cache::etag_key(host, &login, &what))
            .map(|(saved, _)| saved)
    };
    match job {
        Job::LoadRuns { repo, .. } => Job::LoadRuns {
            saved: saved(etag_what(&repo, None)),
            repo,
        },
        Job::LoadNotifications { all, .. } => Job::LoadNotifications {
            saved: saved(notifications_what(all)),
            all,
        },
        Job::LoadJobs { repo, run_id, .. } => Job::LoadJobs {
            saved: saved(etag_what(&repo, Some(run_id))),
            repo,
            run_id,
        },
        other => other,
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
        Message::Runs {
            repo,
            result: Ok((_, Some(saved))),
        } => cache.put(
            &cache::etag_key(host, &login, &etag_what(repo, None)),
            saved,
        ),
        Message::Notifications {
            all,
            result: Ok((_, Some(saved))),
        } => cache.put(
            &cache::etag_key(host, &login, &notifications_what(*all)),
            saved,
        ),
        Message::Jobs {
            repo,
            run_id,
            result: Ok((_, Some(saved))),
        } => cache.put(
            &cache::etag_key(host, &login, &etag_what(repo, Some(*run_id))),
            saved,
        ),
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
            Job::LoadPrCommits { repo, number } => Message::PrCommits {
                number,
                result: client()
                    .and_then(|c| github::history::commits(&c, &repo, number).map_err(text)),
            },
            Job::LoadNotifications { all, saved } => Message::Notifications {
                all,
                result: client()
                    .and_then(|c| github::notifications::list(&c, all, saved).map_err(text)),
            },
            Job::LoadChoices { repo, purpose } => Message::Choices {
                purpose,
                result: client().and_then(|c| {
                    let items = match purpose {
                        picker::Purpose::Labels => {
                            github::choices::labels(&c, &repo).map(|labels| {
                                labels
                                    .into_iter()
                                    .map(|label| picker::Item::new(label.id, label.name))
                                    .collect()
                            })
                        }
                        _ => github::choices::assignable(&c, &repo).map(|logins| {
                            logins
                                .into_iter()
                                .map(|login| picker::Item::new(login.clone(), login))
                                .collect()
                        }),
                    };
                    items.map_err(text)
                }),
            },
            Job::LoadPrTimeline { repo, number } => Message::PrTimeline {
                number,
                result: client()
                    .and_then(|c| github::history::timeline(&c, &repo, number).map_err(text)),
            },
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
            Job::LoadRuns { repo, saved } => Message::Runs {
                result: client()
                    .and_then(|c| github::actions::runs(&c, &repo, saved).map_err(text)),
                repo,
            },
            Job::LoadJobs {
                repo,
                run_id,
                saved,
            } => Message::Jobs {
                result: client()
                    .and_then(|c| github::actions::jobs(&c, &repo, run_id, saved).map_err(text)),
                repo,
                run_id,
            },
            Job::GitWrite(repo, op) => {
                let commit = matches!(op, app::GitOp::Commit(_));
                let result = match op {
                    app::GitOp::Stage(path) => repo.stage(&path).map(|()| String::new()),
                    app::GitOp::Unstage(path) => repo.unstage(&path).map(|()| String::new()),
                    app::GitOp::Commit(message) => repo.commit(&message),
                };
                Message::GitDone {
                    commit,
                    result: result.map_err(text),
                }
            }
            Job::LoadHistory(repo) => Message::History(repo.history(500).map_err(text)),
            Job::LoadChanges(repo) => Message::Changes(
                repo.status()
                    .and_then(|status| {
                        Ok(local::Changes {
                            status,
                            working: repo.working_diff()?,
                            staged: repo.staged_diff()?,
                            author: repo.author().unwrap_or_else(|_| "unknown".into()),
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
