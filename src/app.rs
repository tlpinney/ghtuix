//! Application state and the one dispatch path for keys, clicks and background results.

use crate::actions::{ActionsView, OpenRun};
use crate::compose::{Compose, Stage, Target};
use crate::dashboard::{DashboardView, Target as RecordTarget};
use crate::git::{GraphLine, Repo};
use crate::github::actions::{Job as RunJob, Run, Saved};
use crate::github::dashboard::Metrics;
use crate::github::notifications::Notification;
use crate::github::search::Hit;
use crate::inbox::{Inbox, Tab};
use crate::keybar::{self, Control};
use crate::local::{Changes, Local, LocalTab};
use crate::log::LogView;
use crate::model::{Event, MergeMethod, PrCommit, PrPage, PullRequest, WriteKind};
use crate::notifications::NotificationsView;
use crate::picker::{Item, Picker, Purpose};
use crate::pr::{Anchor, PrTab, PrView};
use crate::search::SearchView;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// The top-level views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Inbox,
    Local,
    Actions,
    Notifications,
    Search,
    Dashboard,
}

impl View {
    const ALL: [(View, char, &'static str); 6] = [
        (View::Inbox, '1', "Inbox"),
        (View::Local, '2', "Local"),
        (View::Actions, '3', "Actions"),
        (View::Notifications, '4', "Notifications"),
        (View::Search, '5', "Search"),
        (View::Dashboard, '6', "Dashboard"),
    ];
}

/// Everything a key or a click can ask the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Show(View),
    Tab(Tab),
    Select(usize),
    Move(isize),
    /// Open the selected PR.
    Open,
    /// Leave the open PR.
    Back,
    PrTab(PrTab),
    /// Put the PR view's cursor on anchor `i` (a diff line or a thread).
    Cursor(usize),
    /// Comment on the diff line under the cursor.
    CommentLine,
    /// Reply in the thread under the cursor.
    ReplyThread,
    /// Resolve or unresolve the thread under the cursor.
    ToggleResolve,
    /// Open the log of the check under the cursor.
    OpenLog,
    /// Jump to the next (`1`) or previous (`-1`) failure in the open log.
    Failure(isize),
    /// Stage the selected file of the Working list.
    Stage,
    /// Unstage the selected file of the Staged list.
    Unstage,
    /// Write a commit message for what is staged.
    StartCommit,
    /// Re-run the failed jobs of the open workflow run.
    Rerun,
    /// Cancel the open workflow run.
    CancelRun,
    /// Start a merge of the open PR.
    Merge,
    /// Pick the merge method in the merge preview.
    MergeMethod(MergeMethod),
    /// Switch the Dashboard between 7 and 30 days.
    DashboardDays(i64),
    /// Put the keys into the search query.
    SearchEdit,
    /// Run the typed search.
    RunSearch,
    /// Show unread notifications only (`false`) or all of them.
    NotificationsAll(bool),
    /// Mark the selected notification read, or done.
    MarkRead,
    MarkDone,
    /// Open the PR actions menu (close, draft, title, labels, reviewers).
    PrMenu,
    /// Click on row `i` of the open picker: choose it, or toggle it in a multi picker.
    PickerRow(usize),
    PickerToggle,
    PickerDone,
    PickerCancel,
    LocalTab(LocalTab),
    /// Start writing a comment or review on the open PR.
    Compose(WriteKind),
    ComposePreview,
    ComposeConfirm,
    /// Back from the preview to editing.
    ComposeEdit,
    ComposeCancel,
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
    Pr(Result<Box<PrPage>, String>),
    /// Workflow runs, with a fresh body and ETag to save when GitHub sent one.
    Runs {
        repo: String,
        result: Result<(Vec<Run>, Option<Saved>), String>,
    },
    /// The jobs of one run.
    Jobs {
        repo: String,
        run_id: i64,
        result: Result<(Vec<RunJob>, Option<Saved>), String>,
    },
    History(Result<Vec<GraphLine>, String>),
    Changes(Result<Changes, String>),
    /// `git show` of one commit.
    Commit {
        sha: String,
        result: Result<String, String>,
    },
    /// A PR's commits or timeline, read when its tab first showed.
    PrCommits {
        number: i64,
        result: Result<Vec<PrCommit>, String>,
    },
    PrTimeline {
        number: i64,
        result: Result<Vec<Event>, String>,
    },
    /// An Actions job's log.
    Log {
        repo: String,
        job_id: i64,
        finished: bool,
        result: Result<String, String>,
    },
    /// A local git write finished; for a commit, the new commit's id.
    GitDone {
        commit: bool,
        result: Result<String, String>,
    },
    /// A write finished: the new comment's or review's URL, or why it failed.
    Written(Result<String, String>),
    Search {
        query: String,
        result: Result<(Vec<Hit>, i64), String>,
    },
    Dashboard {
        repo: String,
        days: i64,
        result: Result<Box<Metrics>, String>,
    },
    Notifications {
        all: bool,
        result: Result<(Vec<Notification>, Option<Saved>), String>,
    },
    /// What a label or reviewer picker offers.
    Choices {
        purpose: Purpose,
        result: Result<Vec<Item>, String>,
    },
    /// The diff between two commits of an open PR.
    Diff {
        base: String,
        head: String,
        result: Result<String, String>,
    },
}

/// What the UI thread asks the background to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    SignIn,
    LoadInbox,
    LoadMerged,
    /// Read a PR page; with `updated_at`, a cached page of that version may answer.
    LoadPr {
        repo: String,
        number: i64,
        updated_at: Option<String>,
    },
    LoadDiff {
        repo: String,
        number: i64,
        base: String,
        head: String,
    },
    LoadPrCommits {
        repo: String,
        number: i64,
    },
    LoadPrTimeline {
        repo: String,
        number: i64,
    },
    /// A repository's PR and CI metrics over the last `days` days.
    LoadDashboard {
        repo: String,
        days: i64,
    },
    /// Run a GitHub issue and pull request search.
    Search(String),
    /// Notification threads; `saved` makes the read conditional.
    LoadNotifications {
        all: bool,
        saved: Option<Saved>,
    },
    /// The repository's labels or assignable users, for a picker.
    LoadChoices {
        repo: String,
        purpose: Purpose,
    },
    /// Download a job's log; a finished job's log may come from the cache.
    LoadLog {
        repo: String,
        job_id: i64,
        finished: bool,
    },
    /// Workflow runs; `saved` (filled from the cache) makes the read conditional.
    LoadRuns {
        repo: String,
        saved: Option<Saved>,
    },
    LoadJobs {
        repo: String,
        run_id: i64,
        saved: Option<Saved>,
    },
    /// Change the local checkout: stage, unstage or commit.
    GitWrite(Repo, GitOp),
    LoadHistory(Repo),
    LoadChanges(Repo),
    ShowCommit(Repo, String),
    Write {
        kind: WriteKind,
        pr_id: String,
        head_oid: String,
        body: String,
    },
    Open(String),
}

/// A local git write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitOp {
    Stage(String),
    Unstage(String),
    Commit(String),
}

pub struct App {
    pub view: View,
    pub inbox: Inbox,
    /// The PR opened from the Inbox (or Notifications), if any.
    pub pr: Option<PrView>,
    /// The view Back returns to from the open PR.
    pr_from: View,
    pub notifications: NotificationsView,
    pub search: SearchView,
    pub dashboard: DashboardView,
    /// A write being composed on the open PR.
    pub compose: Option<Compose>,
    /// A menu or a list of labels or reviewers to choose from.
    pub picker: Option<Picker>,
    pub local: Local,
    /// An Actions job log shown over the current view.
    pub log: Option<LogView>,
    pub actions: ActionsView,
    pub login: Option<String>,
    /// The account the cached Inbox belongs to, until sign-in confirms it.
    cached_login: Option<String>,
    pub status: String,
    /// A rate-limit warning from the GitHub client, shown beside the status.
    pub api_note: Option<String>,
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
            pr: None,
            pr_from: View::Inbox,
            notifications: NotificationsView::default(),
            search: SearchView::default(),
            dashboard: DashboardView::new(Err("No GitHub repository yet".into())),
            compose: None,
            picker: None,
            local: Local::new(Err("No checkout yet".into())),
            log: None,
            actions: ActionsView::new(Err("No GitHub repository yet".into())),
            login: None,
            cached_login: None,
            status: "Signing in…".into(),
            api_note: None,
            quit: false,
            jobs: vec![Job::SignIn],
            hits: Vec::new(),
        }
    }
}

impl App {
    /// The single entry point for every action.
    pub fn dispatch(&mut self, action: Action) {
        // An open log takes scrolling, failure jumps and Back.
        if let Some(log) = &mut self.log {
            match action {
                Action::Move(delta) => return log.scroll_by(delta),
                Action::Failure(direction) => return log.jump(direction),
                Action::Back => {
                    self.log = None;
                    return;
                }
                Action::OpenInBrowser => {
                    if let Some(url) = log.url.clone() {
                        self.jobs.push(Job::Open(url));
                    }
                    return;
                }
                _ => {}
            }
        }
        if let (Some(picker), Action::Move(delta)) = (&mut self.picker, &action) {
            return picker.move_selection(*delta);
        }
        match action {
            Action::Show(view) => {
                self.view = view;
                if view == View::Local && self.local.history.is_none() {
                    self.load_local();
                }
                if view == View::Actions && self.actions.runs.is_none() {
                    self.load_actions();
                }
                if view == View::Notifications && self.notifications.items.is_none() {
                    self.load_notifications();
                }
                if view == View::Dashboard && self.dashboard.metrics.is_none() {
                    self.load_dashboard();
                }
            }
            Action::DashboardDays(days) => {
                if self.dashboard.days != days {
                    self.dashboard.days = days;
                    self.dashboard.metrics = None;
                    self.dashboard.open = None;
                    self.dashboard.loading = false;
                    self.load_dashboard();
                }
            }
            Action::Select(index) if self.view == View::Dashboard => {
                if index == self.dashboard.current() {
                    self.dispatch(Action::Open);
                } else {
                    self.dashboard.select(index);
                }
            }
            Action::Move(delta) if self.view == View::Dashboard => {
                self.dashboard.move_selection(delta)
            }
            Action::Open if self.view == View::Dashboard => match self.dashboard.open {
                None => {
                    if self.dashboard.selected < self.dashboard.cards().len() {
                        self.dashboard.open = Some(self.dashboard.selected);
                        self.dashboard.row = 0;
                    }
                }
                Some(_) => match self.dashboard.selected_record().map(|r| r.target) {
                    Some(RecordTarget::Pr {
                        repo,
                        number,
                        title,
                        url,
                    }) => self.open_pr_from(repo, number, title, url),
                    Some(RecordTarget::Web(url)) => self.jobs.push(Job::Open(url)),
                    None => {}
                },
            },
            Action::Back if self.view == View::Dashboard => self.dashboard.open = None,
            Action::OpenInBrowser if self.view == View::Dashboard => {
                let url = match self.dashboard.selected_record().map(|r| r.target) {
                    Some(RecordTarget::Pr { url, .. } | RecordTarget::Web(url)) => Some(url),
                    None => None,
                };
                if let Some(url) = url {
                    self.jobs.push(Job::Open(url));
                }
            }
            Action::SearchEdit => self.search.editing = true,
            Action::RunSearch => self.run_search(),
            Action::Select(index) if self.view == View::Search => {
                self.search.editing = false;
                if index == self.search.selected {
                    self.dispatch(Action::Open);
                } else {
                    self.search.select(index);
                }
            }
            Action::Move(delta) if self.view == View::Search => {
                self.search.editing = false;
                self.search.move_selection(delta);
            }
            Action::Open if self.view == View::Search => {
                let Some(hit) = self.search.selected_hit().cloned() else {
                    return;
                };
                if hit.pr {
                    self.open_pr_from(hit.repo, hit.number, hit.title, hit.url);
                } else {
                    self.status = format!("Opening {} on the web", hit.reference());
                    self.jobs.push(Job::Open(hit.url));
                }
            }
            Action::OpenInBrowser if self.view == View::Search => {
                if let Some(hit) = self.search.selected_hit() {
                    self.jobs.push(Job::Open(hit.url.clone()));
                }
            }
            Action::NotificationsAll(all) => {
                if self.notifications.all != all {
                    self.notifications.all = all;
                    self.notifications.items = None;
                    self.notifications.selected = 0;
                    self.notifications.loading = false;
                    self.load_notifications();
                }
            }
            Action::Select(index) if self.view == View::Notifications => {
                if index == self.notifications.selected {
                    self.dispatch(Action::Open);
                } else {
                    self.notifications.select(index);
                }
            }
            Action::Move(delta) if self.view == View::Notifications => {
                self.notifications.move_selection(delta)
            }
            Action::Open if self.view == View::Notifications => {
                let Some(item) = self.notifications.selected_item() else {
                    return;
                };
                match item.pr_number() {
                    Some(number) => {
                        let (repo, title, url) = (
                            item.repository.full_name.clone(),
                            item.subject.title.clone(),
                            item.web_url(),
                        );
                        self.open_pr_from(repo, number, title, url);
                    }
                    None => {
                        self.status = format!("Opening {} on the web", item.subject.title);
                        self.jobs.push(Job::Open(item.web_url()));
                    }
                }
            }
            Action::OpenInBrowser if self.view == View::Notifications => {
                if let Some(item) = self.notifications.selected_item() {
                    self.jobs.push(Job::Open(item.web_url()));
                }
            }
            Action::MarkRead | Action::MarkDone => {
                let Some(item) = self.notifications.selected_item() else {
                    return;
                };
                let thread_id = item.id.clone();
                let kind = if action == Action::MarkRead {
                    if !item.unread {
                        self.status = "That notification is already read".into();
                        return;
                    }
                    WriteKind::MarkRead { thread_id }
                } else {
                    WriteKind::MarkDone { thread_id }
                };
                let target = Target::run(&item.repository.full_name, &item.subject.title);
                self.compose = Some(Compose::new(kind, target));
            }
            Action::LocalTab(tab) => {
                self.local.tab = tab;
                self.local.commit = None;
                self.local.scroll = 0;
            }
            Action::Select(index)
                if self.view == View::Local && self.local.tab != LocalTab::History =>
            {
                self.local.select_file(index);
            }
            Action::Move(delta)
                if self.view == View::Local && self.local.tab != LocalTab::History =>
            {
                // Arrows move between files; paging and the wheel scroll the diff.
                if delta.abs() == 1 && self.local.commit.is_none() {
                    self.local
                        .select_file(self.local.file.saturating_add_signed(delta));
                } else {
                    self.local.scroll_by(delta);
                }
            }
            Action::Stage | Action::Unstage => {
                if let (Ok(repo), Some((path, _))) = (&self.local.repo, self.local.selected_file())
                {
                    let op = if action == Action::Stage {
                        GitOp::Stage(path.clone())
                    } else {
                        GitOp::Unstage(path.clone())
                    };
                    self.jobs.push(Job::GitWrite(repo.clone(), op));
                }
            }
            Action::StartCommit => match &self.local.changes {
                Some(Ok(changes)) if !changes.status.staged.is_empty() => {
                    let root = self
                        .local
                        .repo
                        .as_ref()
                        .map_or(String::new(), |repo| repo.root.display().to_string());
                    let kind = WriteKind::Commit {
                        branch: changes.status.branch.clone(),
                        author: changes.author.clone(),
                        files: changes
                            .status
                            .staged
                            .iter()
                            .map(|(path, code)| format!("{code} {path}"))
                            .collect(),
                    };
                    self.compose = Some(Compose::new(kind, Target::run(&root, "")));
                }
                _ => self.status = "Nothing is staged to commit; + stages a file".into(),
            },
            Action::Select(index) if self.view == View::Local => {
                if index == self.local.selected {
                    self.dispatch(Action::Open);
                } else {
                    self.local.selected = index;
                }
            }
            Action::Move(delta) if self.view == View::Local => {
                if self.local.tab == LocalTab::History && self.local.commit.is_none() {
                    self.local.move_selection(delta);
                } else {
                    self.local.scroll_by(delta);
                }
            }
            Action::Open if self.view == View::Local => {
                if let (Ok(repo), Some(sha)) = (&self.local.repo, self.local.selected_sha()) {
                    self.jobs.push(Job::ShowCommit(repo.clone(), sha.clone()));
                    self.local.commit = Some((sha, None));
                    self.local.scroll = 0;
                }
            }
            Action::Back if self.view == View::Local => self.local.commit = None,
            // The Actions view: the run list, then one run's jobs.
            Action::Select(index) if self.view == View::Actions => {
                if index == self.actions.current() {
                    self.dispatch(Action::Open);
                } else {
                    self.actions.select(index);
                }
            }
            Action::Move(delta) if self.view == View::Actions => self.actions.move_selection(delta),
            Action::Open if self.view == View::Actions => match &self.actions.open {
                None => {
                    if let (Ok(repo), Some(run)) = (&self.actions.repo, self.actions.selected_run())
                    {
                        self.jobs.push(Job::LoadJobs {
                            repo: repo.clone(),
                            run_id: run.id,
                            saved: None,
                        });
                        self.actions.open = Some(OpenRun {
                            run: run.clone(),
                            jobs: None,
                            selected: 0,
                        });
                    }
                }
                Some(_) => {
                    if let (Ok(repo), Some(job)) = (&self.actions.repo, self.actions.selected_job())
                    {
                        let finished = job.status == "completed";
                        self.jobs.push(Job::LoadLog {
                            repo: repo.clone(),
                            job_id: job.id,
                            finished,
                        });
                        self.log =
                            Some(LogView::new(job.name.clone(), job.html_url.clone(), job.id));
                    }
                }
            },
            Action::Back if self.view == View::Actions => self.actions.open = None,
            Action::Rerun | Action::CancelRun => {
                let (Ok(repo), Some(open)) = (&self.actions.repo, &self.actions.open) else {
                    return;
                };
                let run = format!(
                    "{} #{}",
                    open.run.name.clone().unwrap_or_default(),
                    open.run.run_number
                );
                let kind = if action == Action::Rerun {
                    WriteKind::Rerun {
                        repo: repo.clone(),
                        run_id: open.run.id,
                        run: run.clone(),
                    }
                } else {
                    WriteKind::CancelRun {
                        repo: repo.clone(),
                        run_id: open.run.id,
                        run: run.clone(),
                    }
                };
                let target = Target::run(repo, &open.run.display_title);
                self.compose = Some(Compose::new(kind, target));
            }
            Action::Tab(tab) => {
                self.inbox.tab = tab;
                self.inbox.select(0);
                if tab == Tab::Merged && self.inbox.merged.prs.is_none() {
                    self.load(Job::LoadMerged);
                }
            }
            // Clicking the row that is already selected opens it.
            Action::Select(index) if index == self.inbox.selected => self.dispatch(Action::Open),
            Action::Select(index) => self.inbox.select(index),
            Action::Move(delta) => match &mut self.pr {
                Some(pr) => pr.move_cursor(delta),
                None => self.inbox.move_selection(delta),
            },
            Action::Cursor(index) => {
                if let Some(pr) = &mut self.pr {
                    if pr.cursor == index {
                        // A click on the line already under the cursor acts on it.
                        match pr.tab {
                            PrTab::Files => self.dispatch(Action::CommentLine),
                            PrTab::Checks => self.dispatch(Action::OpenLog),
                            _ => self.dispatch(Action::ReplyThread),
                        }
                    } else {
                        pr.cursor = index;
                    }
                }
            }
            Action::OpenLog => {
                let Some(pr) = &self.pr else { return };
                let (Some(page), Some(Anchor::Check(index))) = (&pr.page, pr.anchor()) else {
                    return;
                };
                let Some(check) = page.checks.get(index) else {
                    return;
                };
                match check.job_id {
                    Some(job_id) => {
                        let finished = check.state != crate::model::Checks::Pending;
                        self.jobs.push(Job::LoadLog {
                            repo: page.repo.clone(),
                            job_id,
                            finished,
                        });
                        self.log =
                            Some(LogView::new(check.name.clone(), check.url.clone(), job_id));
                    }
                    None => {
                        self.status =
                            format!("{} is not a GitHub Actions job; w opens it", check.name);
                    }
                }
            }
            Action::Failure(_) => {}
            Action::Merge => {
                let Some(page) = self.pr.as_ref().and_then(|pr| pr.page.as_ref()) else {
                    self.status = "Wait for the PR to load before merging".into();
                    return;
                };
                let refusal = if page.state != "open" {
                    Some(format!(
                        "This PR is {}; there is nothing to merge",
                        page.state
                    ))
                } else if page.draft {
                    Some("A draft PR cannot be merged; mark it ready for review first".into())
                } else if page.merge_methods.is_empty() {
                    Some("This repository allows no merge method".into())
                } else {
                    None
                };
                match refusal {
                    Some(reason) => self.status = reason,
                    None => {
                        let method = page.merge_methods[0];
                        let target = Target::of(page);
                        self.compose = Some(Compose::new(WriteKind::Merge { method }, target));
                    }
                }
            }
            Action::MergeMethod(method) => {
                if let Some(compose) = &mut self.compose
                    && matches!(compose.kind, WriteKind::Merge { .. })
                    && compose.target.merge_methods.contains(&method)
                {
                    compose.kind = WriteKind::Merge { method };
                }
            }
            Action::PrMenu => {
                let Some(page) = self.pr.as_ref().and_then(|pr| pr.page.as_ref()) else {
                    self.status = "Wait for the PR to load first".into();
                    return;
                };
                let mut items = Vec::new();
                match page.state.as_str() {
                    "open" if page.draft => items.push(Item::new("ready", "Ready for review")),
                    "open" => items.push(Item::new("draft", "Convert to draft")),
                    _ => {}
                }
                match page.state.as_str() {
                    "open" => items.push(Item::new("close", "Close")),
                    "closed" => items.push(Item::new("reopen", "Reopen")),
                    _ => {}
                }
                items.push(Item::new("title", "Edit title"));
                items.push(Item::new("labels", "Labels…"));
                items.push(Item::new("reviewers", "Request reviewers…"));
                let mut picker = Picker::new(Purpose::PrMenu, "PR actions", false);
                picker.items = Some(Ok(items));
                self.picker = Some(picker);
            }
            Action::PickerRow(row) => match &mut self.picker {
                Some(picker) if picker.multi => picker.toggle(row),
                Some(picker) => {
                    picker.selected = row;
                    self.dispatch(Action::PickerDone);
                }
                None => {}
            },
            Action::PickerToggle => {
                if let Some(picker) = &mut self.picker
                    && picker.multi
                {
                    let row = picker.selected;
                    picker.toggle(row);
                }
            }
            Action::PickerCancel => self.picker = None,
            Action::PickerDone => self.picker_done(),
            Action::CommentLine | Action::ReplyThread | Action::ToggleResolve => {
                if let Some(kind) = self.cursor_write(&action) {
                    self.dispatch(Action::Compose(kind));
                }
            }
            Action::Open => {
                if let Some(pr) = self.inbox.selected_pr().cloned() {
                    self.jobs.push(Job::LoadPr {
                        repo: pr.repo.clone(),
                        number: pr.number,
                        updated_at: Some(pr.updated_at.clone()),
                    });
                    self.pr = Some(PrView::new(pr));
                }
            }
            Action::Back => {
                if self.pr.take().is_some() {
                    self.view = std::mem::replace(&mut self.pr_from, View::Inbox);
                }
            }
            Action::Compose(kind) => match self.pr.as_ref().and_then(|pr| pr.page.as_ref()) {
                Some(page) => {
                    let mut compose = Compose::new(kind, Target::of(page));
                    if let WriteKind::Title { from } = &compose.kind {
                        let from = from.clone();
                        compose.editor.insert(&from);
                    }
                    self.picker = None;
                    self.compose = Some(compose);
                }
                None => self.status = "Wait for the PR to load before writing".into(),
            },
            Action::ComposePreview => {
                if let Some(compose) = &mut self.compose {
                    compose.preview();
                }
            }
            Action::ComposeEdit => match &mut self.compose {
                // A write without text has nothing to edit: back means cancel.
                Some(compose) if !compose.kind.has_text() => self.compose = None,
                Some(compose) => compose.stage = Stage::Editing,
                None => {}
            },
            Action::ComposeCancel => self.compose = None,
            Action::ComposeConfirm => {
                if let Some(compose) = &mut self.compose
                    && compose.stage == Stage::Preview
                {
                    compose.stage = Stage::Sending;
                    compose.error = None;
                    if matches!(compose.kind, WriteKind::Commit { .. }) {
                        if let Ok(repo) = &self.local.repo {
                            let message = compose.editor.text();
                            self.jobs
                                .push(Job::GitWrite(repo.clone(), GitOp::Commit(message)));
                        }
                        return;
                    }
                    self.jobs.push(Job::Write {
                        kind: compose.kind.clone(),
                        pr_id: compose.target.pr_id.clone(),
                        head_oid: compose.target.head_oid.clone(),
                        body: compose.editor.text(),
                    });
                }
            }
            Action::PrTab(tab) => {
                if let Some(pr) = &mut self.pr {
                    pr.switch(tab);
                    // Commits and the timeline are read only when their tab is looked at.
                    let (repo, number) = (pr.summary.repo.clone(), pr.summary.number);
                    if tab == PrTab::Commits && pr.commits.is_none() {
                        self.jobs.push(Job::LoadPrCommits { repo, number });
                    } else if tab == PrTab::Timeline && pr.timeline.is_none() {
                        self.jobs.push(Job::LoadPrTimeline { repo, number });
                    }
                }
                self.load_diff_if_needed();
            }
            Action::OpenInBrowser if self.view == View::Actions => {
                let url = match &self.actions.open {
                    Some(open) => Some(open.run.html_url.clone()),
                    None => self.actions.selected_run().map(|run| run.html_url.clone()),
                };
                if let Some(url) = url {
                    self.jobs.push(Job::Open(url));
                }
            }
            Action::OpenInBrowser => {
                let target = match &self.pr {
                    Some(pr) => Some((pr.summary.reference(), pr.url().to_owned())),
                    None => self
                        .inbox
                        .selected_pr()
                        .map(|pr| (pr.reference(), pr.url.clone())),
                };
                match target {
                    Some((reference, url)) => {
                        self.status = format!("Opening {reference} in the browser");
                        self.jobs.push(Job::Open(url));
                    }
                    None => self.status = "Select a pull request first".into(),
                }
            }
            Action::Refresh => {
                if self.view == View::Actions {
                    self.load_actions();
                } else if self.view == View::Notifications {
                    self.load_notifications();
                } else if self.view == View::Dashboard {
                    self.load_dashboard();
                } else if self.view == View::Search {
                    // Only the last search runs again; half-typed text never does.
                    if let Some(query) = self.search.ran.clone()
                        && !self.search.editing
                        && !self.search.loading
                    {
                        self.search.loading = true;
                        self.jobs.push(Job::Search(query));
                    }
                } else if self.view == View::Inbox
                    && let Some(pr) = &mut self.pr
                {
                    if !pr.loading {
                        pr.loading = true;
                        pr.error = None;
                        self.jobs.push(Job::LoadPr {
                            repo: pr.summary.repo.clone(),
                            number: pr.summary.number,
                            updated_at: None,
                        });
                    }
                } else if self.view == View::Local {
                    self.load_local();
                } else if self.login.is_none() {
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

    /// Acts on the open picker's choice: a menu item, or the checked labels or reviewers.
    fn picker_done(&mut self) {
        let Some(picker) = &self.picker else { return };
        let Some(page) = self.pr.as_ref().and_then(|pr| pr.page.as_ref()) else {
            self.picker = None;
            return;
        };
        match picker.purpose {
            Purpose::PrMenu => {
                let Some(id) = picker.current().map(|item| item.id.clone()) else {
                    return;
                };
                self.picker = None;
                let kind = match id.as_str() {
                    "close" => WriteKind::Close,
                    "reopen" => WriteKind::Reopen,
                    "draft" => WriteKind::ToDraft,
                    "ready" => WriteKind::Ready,
                    "title" => WriteKind::Title {
                        from: page.title.clone(),
                    },
                    "labels" | "reviewers" => {
                        let (purpose, title) = if id == "labels" {
                            (Purpose::Labels, "Labels")
                        } else {
                            (Purpose::Reviewers, "Request reviewers")
                        };
                        self.jobs.push(Job::LoadChoices {
                            repo: page.repo.clone(),
                            purpose,
                        });
                        self.picker = Some(Picker::new(purpose, title, true));
                        return;
                    }
                    _ => return,
                };
                self.dispatch(Action::Compose(kind));
            }
            Purpose::Labels => {
                if picker.items.as_ref().is_none_or(|items| items.is_err()) {
                    return;
                }
                let checked: Vec<&Item> = picker.checked();
                let add: Vec<crate::model::Label> = checked
                    .iter()
                    .filter(|item| !page.labels.iter().any(|l| l.id == item.id))
                    .map(|item| crate::model::Label {
                        id: item.id.clone(),
                        name: item.label.clone(),
                    })
                    .collect();
                let remove: Vec<crate::model::Label> = page
                    .labels
                    .iter()
                    .filter(|label| !checked.iter().any(|item| item.id == label.id))
                    .cloned()
                    .collect();
                self.picker = None;
                if add.is_empty() && remove.is_empty() {
                    self.status = "The labels are unchanged".into();
                } else {
                    self.dispatch(Action::Compose(WriteKind::Labels { add, remove }));
                }
            }
            Purpose::Reviewers => {
                if picker.items.as_ref().is_none_or(|items| items.is_err()) {
                    return;
                }
                let checked: Vec<String> = picker
                    .checked()
                    .iter()
                    .map(|item| item.id.clone())
                    .collect();
                let add: Vec<String> = checked
                    .iter()
                    .filter(|login| !page.requested.contains(login))
                    .cloned()
                    .collect();
                let remove: Vec<String> = page
                    .requested
                    .iter()
                    .filter(|login| !checked.contains(login))
                    .cloned()
                    .collect();
                let (repo, number) = (page.repo.clone(), page.number);
                self.picker = None;
                if add.is_empty() && remove.is_empty() {
                    self.status = "The review requests are unchanged".into();
                } else {
                    self.dispatch(Action::Compose(WriteKind::Reviewers {
                        repo,
                        number,
                        add,
                        remove,
                    }));
                }
            }
        }
    }

    /// Shows `login`'s cached Inbox (saved at `saved_at`) until sign-in and a refresh.
    pub fn restore(&mut self, login: String, prs: Vec<PullRequest>, saved_at: i64) {
        self.inbox.open.prs = Some(prs);
        self.inbox.open.fetched_at = Some(saved_at);
        self.inbox.open.cached = true;
        self.cached_login = Some(login);
    }

    /// The write `action` means for the anchor under the PR view's cursor, if it fits.
    fn cursor_write(&self, action: &Action) -> Option<WriteKind> {
        let pr = self.pr.as_ref()?;
        let page = pr.page.as_ref()?;
        match (action, pr.anchor()?) {
            (Action::CommentLine, Anchor::Line(place)) => Some(WriteKind::LineComment {
                path: place.path,
                line: place.line,
                side: place.side,
            }),
            (Action::ReplyThread, Anchor::Thread(index)) => {
                let thread = page.threads.get(index)?;
                Some(WriteKind::Reply {
                    thread_id: thread.id.clone(),
                    place: place(thread),
                })
            }
            (Action::ToggleResolve, Anchor::Thread(index)) => {
                let thread = page.threads.get(index)?;
                Some(WriteKind::Resolve {
                    thread_id: thread.id.clone(),
                    place: place(thread),
                    resolve: !thread.resolved,
                })
            }
            _ => None,
        }
    }

    /// Reads the runs again, and the open run's jobs, unless a read is running.
    fn load_actions(&mut self) {
        if let Ok(repo) = &self.actions.repo
            && !self.actions.loading
        {
            self.actions.loading = true;
            self.jobs.push(Job::LoadRuns {
                repo: repo.clone(),
                saved: None,
            });
            if let Some(open) = &self.actions.open {
                self.jobs.push(Job::LoadJobs {
                    repo: repo.clone(),
                    run_id: open.run.id,
                    saved: None,
                });
            }
        }
    }

    /// Opens PR `repo#number` in the PR view; Back returns to the current view.
    fn open_pr_from(&mut self, repo: String, number: i64, title: String, url: String) {
        let summary = PullRequest {
            repo,
            number,
            title,
            url,
            body: String::new(),
            draft: false,
            author: crate::model::Person {
                login: String::new(),
                bot: false,
            },
            created_at: String::new(),
            updated_at: String::new(),
            merged_at: None,
            queues: Vec::new(),
            decision: crate::model::Decision::None,
            checks: crate::model::Checks::None,
            reviewers: Vec::new(),
        };
        // Without the PR's own updatedAt, no cached page can be trusted to be current.
        self.jobs.push(Job::LoadPr {
            repo: summary.repo.clone(),
            number,
            updated_at: None,
        });
        self.pr = Some(PrView::new(summary));
        self.pr_from = self.view;
        self.view = View::Inbox;
    }

    /// Reads the Dashboard's numbers again, unless a read is running.
    fn load_dashboard(&mut self) {
        if let Ok(repo) = &self.dashboard.repo
            && !self.dashboard.loading
        {
            self.dashboard.loading = true;
            self.jobs.push(Job::LoadDashboard {
                repo: repo.clone(),
                days: self.dashboard.days,
            });
        }
    }

    /// Runs the typed query, unless it is empty or the same query is running.
    fn run_search(&mut self) {
        let query = self.search.query.trim().to_owned();
        if query.is_empty() {
            self.status = "Type a search first".into();
            return;
        }
        if self.search.loading && self.search.ran.as_deref() == Some(query.as_str()) {
            return;
        }
        self.search.loading = true;
        self.search.editing = false;
        self.search.selected = 0;
        self.search.ran = Some(query.clone());
        self.jobs.push(Job::Search(query));
    }

    /// Reads the notifications again, unless a read is running.
    fn load_notifications(&mut self) {
        if !self.notifications.loading {
            self.notifications.loading = true;
            self.jobs.push(Job::LoadNotifications {
                all: self.notifications.all,
                saved: None,
            });
        }
    }

    /// Reads the checkout's history and changes again, unless a read is running.
    fn load_local(&mut self) {
        if let Ok(repo) = &self.local.repo
            && !self.local.loading
        {
            self.local.loading = true;
            self.jobs.push(Job::LoadHistory(repo.clone()));
            self.jobs.push(Job::LoadChanges(repo.clone()));
        }
    }

    /// Asks for the open PR's diff when its Files tab shows and the diff is missing.
    fn load_diff_if_needed(&mut self) {
        let Some(pr) = &mut self.pr else { return };
        let Some(page) = &pr.page else { return };
        if pr.tab == PrTab::Files && pr.diff.is_none() && !pr.diff_loading {
            pr.diff_loading = true;
            self.jobs.push(Job::LoadDiff {
                repo: page.repo.clone(),
                number: page.number,
                base: page.base_oid.clone(),
                head: page.head_oid.clone(),
            });
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
                // Another account's cached data is never shown under this one.
                if self
                    .cached_login
                    .take()
                    .is_some_and(|cached| cached != login)
                {
                    self.inbox.open = Default::default();
                    self.inbox.merged = Default::default();
                }
                self.status = format!("Signed in as @{login}");
                self.login = Some(login);
                self.load(Job::LoadInbox);
            }
            Message::SignedIn(Err(error)) => self.status = error,
            Message::Inbox(result) => self.inbox.open.finish(result),
            Message::Merged(result) => self.inbox.merged.finish(result),
            Message::Pr(result) => {
                if let Some(pr) = &mut self.pr {
                    pr.loading = false;
                    match result {
                        Ok(page)
                            if page.repo == pr.summary.repo && page.number == pr.summary.number =>
                        {
                            // A new head or base makes the old diff wrong.
                            let same = pr.page.as_ref().is_some_and(|old| {
                                old.head_oid == page.head_oid && old.base_oid == page.base_oid
                            });
                            if !same {
                                pr.diff = None;
                            }
                            // A newer version of the PR makes the history stale.
                            if pr
                                .page
                                .as_ref()
                                .is_some_and(|old| old.updated_at != page.updated_at)
                            {
                                pr.commits = None;
                                pr.timeline = None;
                            }
                            pr.page = Some(*page);
                            pr.error = None;
                        }
                        Ok(_) => {}
                        Err(error) => pr.error = Some(error),
                    }
                }
                self.load_diff_if_needed();
            }
            Message::Runs { result, .. } => {
                self.actions.loading = false;
                self.actions.runs = Some(result.map(|(runs, _)| runs));
                self.actions.move_selection(0);
                // The open run's summary follows the newest copy of the list.
                if let (Some(open), Some(Ok(runs))) = (&mut self.actions.open, &self.actions.runs)
                    && let Some(fresh) = runs.iter().find(|run| run.id == open.run.id)
                {
                    open.run = fresh.clone();
                }
            }
            Message::Jobs { run_id, result, .. } => {
                if let Some(open) = &mut self.actions.open
                    && open.run.id == run_id
                {
                    let count = result.as_ref().map_or(0, |(jobs, _)| jobs.len());
                    open.jobs = Some(result.map(|(jobs, _)| jobs));
                    open.selected = open.selected.min(count.saturating_sub(1));
                }
            }
            Message::History(result) => {
                self.local.loading = false;
                self.local.history = Some(result);
                self.local.move_selection(0);
            }
            Message::Changes(result) => self.local.changes = Some(result),
            Message::Commit { sha, result } => {
                if let Some((open, shown)) = &mut self.local.commit
                    && *open == sha
                {
                    *shown = Some(result.unwrap_or_else(|error| error));
                }
            }
            Message::PrCommits { number, result } => {
                if let Some(pr) = &mut self.pr
                    && pr.summary.number == number
                {
                    pr.commits = Some(result);
                }
            }
            Message::PrTimeline { number, result } => {
                if let Some(pr) = &mut self.pr
                    && pr.summary.number == number
                {
                    pr.timeline = Some(result);
                }
            }
            Message::Log { job_id, result, .. } => {
                if let Some(log) = &mut self.log
                    && log.job_id == job_id
                {
                    log.load(result);
                }
            }
            Message::GitDone { commit, result } => {
                match (&result, commit) {
                    (Ok(sha), true) => {
                        self.compose = None;
                        self.status = format!("Committed {}", &sha[..sha.len().min(8)]);
                    }
                    (Err(error), true) => {
                        if let Some(compose) = &mut self.compose {
                            compose.stage = Stage::Preview;
                            compose.error = Some(error.clone());
                        }
                    }
                    (Err(error), false) => self.status = error.clone(),
                    (Ok(_), false) => {}
                }
                // Status, diffs and (after a commit) history all changed.
                self.local.loading = false;
                self.load_local();
            }
            Message::Written(Ok(url)) => {
                let kind = self.compose.take().map(|c| c.kind);
                let label = kind.as_ref().map_or("Write".into(), WriteKind::label);
                self.status = format!("{label} published: {url}");
                self.dispatch(Action::Refresh);
                if matches!(
                    kind,
                    Some(WriteKind::Rerun { .. } | WriteKind::CancelRun { .. })
                ) {
                    self.load_actions();
                }
                if matches!(
                    kind,
                    Some(
                        WriteKind::Close
                            | WriteKind::Reopen
                            | WriteKind::ToDraft
                            | WriteKind::Ready
                            | WriteKind::Title { .. }
                    )
                ) {
                    // The Inbox shows state, drafts and titles.
                    self.load(Job::LoadInbox);
                }
                if matches!(kind, Some(WriteKind::Merge { .. })) {
                    // The PR leaves the open queues and joins the merged list.
                    self.inbox.merged = Default::default();
                    self.load(Job::LoadInbox);
                }
            }
            Message::Written(Err(error)) => {
                // The text stays so nothing typed is lost.
                if let Some(compose) = &mut self.compose {
                    compose.stage = Stage::Preview;
                    compose.error = Some(error);
                }
            }
            Message::Dashboard { repo, days, result } => {
                if self.dashboard.repo.as_deref() == Ok(repo.as_str())
                    && self.dashboard.days == days
                {
                    self.dashboard.loading = false;
                    // A failed refresh keeps the numbers already shown.
                    match (result, &self.dashboard.metrics) {
                        (Err(error), Some(Ok(_))) => self.status = error,
                        (result, _) => self.dashboard.metrics = Some(result.map(|m| *m)),
                    }
                    self.dashboard.move_selection(0);
                }
            }
            Message::Search { query, result } => {
                if self.search.ran.as_deref() == Some(query.as_str()) {
                    self.search.loading = false;
                    // Hidden repositories stay hidden here too.
                    let hidden = &self.inbox.hidden;
                    self.search.results = Some(result.map(|(hits, total)| {
                        let shown = hits
                            .into_iter()
                            .filter(|hit| !hidden.contains(&hit.repo))
                            .collect();
                        (shown, total)
                    }));
                    self.search.move_selection(0);
                }
            }
            Message::Notifications { all, result } => {
                if all == self.notifications.all {
                    self.notifications.loading = false;
                    self.notifications.items = Some(result.map(|(items, _)| items));
                    self.notifications.move_selection(0);
                }
            }
            Message::Choices { purpose, result } => {
                let page = self.pr.as_ref().and_then(|pr| pr.page.as_ref());
                if let (Some(picker), Some(page)) = (&mut self.picker, page)
                    && picker.purpose == purpose
                {
                    picker.items = Some(result.map(|items| {
                        items
                            .into_iter()
                            .map(|mut item| {
                                item.checked = match purpose {
                                    Purpose::Labels => page.labels.iter().any(|l| l.id == item.id),
                                    _ => page.requested.contains(&item.id),
                                };
                                item
                            })
                            .collect()
                    }));
                }
            }
            Message::Diff { base, head, result } => {
                if let Some(pr) = &mut self.pr
                    && pr
                        .page
                        .as_ref()
                        .is_some_and(|page| page.base_oid == base && page.head_oid == head)
                {
                    pr.diff_loading = false;
                    match result {
                        Ok(diff) => pr.diff = Some(diff),
                        Err(error) => pr.diff = Some(format!("The diff could not load: {error}")),
                    }
                }
            }
        }
        self.inbox.move_selection(0);
    }

    pub fn key(&mut self, key: KeyEvent) {
        if self.view == View::Search
            && self.search.editing
            && self.compose.is_none()
            && self.picker.is_none()
            && self.log.is_none()
        {
            let ctrl = key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Enter => self.dispatch(Action::RunSearch),
                KeyCode::Esc | KeyCode::Down => self.search.editing = false,
                KeyCode::Backspace => {
                    self.search.query.pop();
                }
                KeyCode::Char('u') if ctrl => self.search.query.clear(),
                KeyCode::Char(c) if !ctrl => self.search.query.push(c),
                _ => {}
            }
            return;
        }
        if let Some(picker) = &mut self.picker
            && self.compose.is_none()
        {
            let ctrl = key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL);
            let action = match key.code {
                KeyCode::Char('c') if ctrl => Some(Action::PickerCancel),
                KeyCode::Esc => Some(Action::PickerCancel),
                KeyCode::Enter => Some(Action::PickerDone),
                KeyCode::Down => Some(Action::Move(1)),
                KeyCode::Up => Some(Action::Move(-1)),
                KeyCode::PageDown => Some(Action::Move(10)),
                KeyCode::PageUp => Some(Action::Move(-10)),
                KeyCode::Char(' ') if picker.multi => Some(Action::PickerToggle),
                KeyCode::Backspace => {
                    picker.backspace();
                    None
                }
                KeyCode::Char(c) if !ctrl => {
                    picker.type_char(c);
                    None
                }
                _ => None,
            };
            if let Some(action) = action {
                self.dispatch(action);
            }
            return;
        }
        if let Some(compose) = &mut self.compose {
            let ctrl = key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL);
            let action = match (compose.stage, key.code) {
                (_, KeyCode::Char('c')) if ctrl => Some(Action::ComposeCancel),
                (Stage::Editing, KeyCode::Char('s')) if ctrl => Some(Action::ComposePreview),
                (Stage::Editing, KeyCode::Esc) => Some(Action::ComposeCancel),
                (Stage::Editing, _) => {
                    compose.editor.key(key);
                    None
                }
                (Stage::Preview, KeyCode::Enter) => Some(Action::ComposeConfirm),
                (Stage::Preview, KeyCode::Char(key))
                    if matches!(compose.kind, WriteKind::Merge { .. }) =>
                {
                    MergeMethod::ALL
                        .iter()
                        .find(|(_, k, _)| *k == key)
                        .map(|(method, ..)| Action::MergeMethod(*method))
                }
                (Stage::Preview, KeyCode::Char('s')) if ctrl => Some(Action::ComposeConfirm),
                (Stage::Preview, KeyCode::Esc) => Some(Action::ComposeEdit),
                _ => None,
            };
            if let Some(action) = action {
                self.dispatch(action);
            }
            return;
        }
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

    /// Pasted text goes into the compose box when it is being edited.
    pub fn paste(&mut self, text: &str) {
        if let Some(compose) = &mut self.compose
            && compose.stage == Stage::Editing
        {
            compose.editor.insert(text);
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
        if self.log.is_some() {
            controls.push(Control::new(
                KeyCode::Char('>'),
                "Next failure",
                Action::Failure(1),
            ));
            controls.push(Control::new(
                KeyCode::Char('<'),
                "Previous",
                Action::Failure(-1),
            ));
            controls.push(Control::new(KeyCode::Char('b'), "Back", Action::Back));
            controls.push(Control::hidden(KeyCode::Left, Action::Back));
            controls.push(Control::hidden(KeyCode::Esc, Action::Back));
            controls.push(Control::new(
                KeyCode::Char('w'),
                "Web",
                Action::OpenInBrowser,
            ));
            controls.push(Control::new(KeyCode::Char('q'), "Quit", Action::Quit));
            return controls;
        }
        if self.view == View::Inbox && self.pr.is_some() {
            controls.push(Control::new(KeyCode::Char('b'), "Back", Action::Back));
            controls.push(Control::hidden(KeyCode::Left, Action::Back));
            controls.push(Control::hidden(KeyCode::Esc, Action::Back));
            controls.push(Control::new(
                KeyCode::Char('w'),
                "Web",
                Action::OpenInBrowser,
            ));
            controls.push(Control::new(
                KeyCode::Char('n'),
                "Comment",
                Action::Compose(WriteKind::Comment),
            ));
            controls.push(Control::new(
                KeyCode::Char('A'),
                "Approve",
                Action::Compose(WriteKind::Approve),
            ));
            controls.push(Control::new(
                KeyCode::Char('X'),
                "Request changes",
                Action::Compose(WriteKind::RequestChanges),
            ));
            controls.push(Control::new(KeyCode::Char('M'), "Merge", Action::Merge));
            controls.push(Control::new(
                KeyCode::Char('p'),
                "PR actions",
                Action::PrMenu,
            ));
            match self.pr.as_ref().and_then(|pr| Some((pr.tab, pr.anchor()?))) {
                Some((PrTab::Files, Anchor::Line(_))) => controls.insert(
                    0,
                    Control::new(KeyCode::Char('i'), "Comment on line", Action::CommentLine),
                ),
                Some((PrTab::Checks, Anchor::Check(_))) => {
                    controls.insert(0, Control::new(KeyCode::Enter, "Log", Action::OpenLog))
                }
                Some((PrTab::Conversation, Anchor::Thread(index))) => {
                    let resolved = self
                        .pr
                        .as_ref()
                        .and_then(|pr| pr.page.as_ref()?.threads.get(index))
                        .is_some_and(|thread| thread.resolved);
                    let label = if resolved { "Unresolve" } else { "Resolve" };
                    controls.insert(
                        0,
                        Control::new(KeyCode::Char('R'), "Reply", Action::ReplyThread),
                    );
                    controls.insert(
                        1,
                        Control::new(KeyCode::Char('Z'), label, Action::ToggleResolve),
                    );
                }
                _ => {}
            }
            controls.extend(
                PrTab::ALL
                    .iter()
                    .map(|(tab, key, _)| Control::hidden(KeyCode::Char(*key), Action::PrTab(*tab))),
            );
        } else if self.view == View::Inbox {
            controls.push(Control::new(KeyCode::Enter, "Open", Action::Open));
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
        } else if self.view == View::Dashboard {
            match self.dashboard.open {
                Some(_) => {
                    controls.push(Control::new(KeyCode::Enter, "Open", Action::Open));
                    controls.push(Control::new(KeyCode::Char('b'), "Back", Action::Back));
                    controls.push(Control::hidden(KeyCode::Left, Action::Back));
                    controls.push(Control::hidden(KeyCode::Esc, Action::Back));
                    controls.push(Control::new(
                        KeyCode::Char('w'),
                        "Web",
                        Action::OpenInBrowser,
                    ));
                }
                None => controls.push(Control::new(KeyCode::Enter, "Records", Action::Open)),
            }
            let (label, days) = if self.dashboard.days == 7 {
                ("30 days", 30)
            } else {
                ("7 days", 7)
            };
            controls.push(Control::new(
                KeyCode::Char('d'),
                label,
                Action::DashboardDays(days),
            ));
        } else if self.view == View::Search && self.search.editing {
            // Keys type into the query; Esc leaves it. Clicks still reach every control.
            controls.push(Control::new(KeyCode::Enter, "Search", Action::RunSearch));
        } else if self.view == View::Search {
            controls.push(Control::new(KeyCode::Enter, "Open", Action::Open));
            controls.push(Control::new(
                KeyCode::Char('/'),
                "Edit search",
                Action::SearchEdit,
            ));
            controls.push(Control::new(
                KeyCode::Char('w'),
                "Web",
                Action::OpenInBrowser,
            ));
        } else if self.view == View::Notifications {
            controls.push(Control::new(KeyCode::Enter, "Open", Action::Open));
            controls.push(Control::new(
                KeyCode::Char('x'),
                "Mark read",
                Action::MarkRead,
            ));
            controls.push(Control::new(KeyCode::Char('D'), "Done", Action::MarkDone));
            let (key, label, all) = if self.notifications.all {
                ('u', "Unread only", false)
            } else {
                ('a', "All", true)
            };
            controls.push(Control::new(
                KeyCode::Char(key),
                label,
                Action::NotificationsAll(all),
            ));
            controls.push(Control::new(
                KeyCode::Char('w'),
                "Web",
                Action::OpenInBrowser,
            ));
        } else if self.view == View::Actions {
            if self.actions.open.is_some() {
                controls.push(Control::new(KeyCode::Enter, "Log", Action::Open));
                controls.push(Control::new(
                    KeyCode::Char('R'),
                    "Re-run failed",
                    Action::Rerun,
                ));
                controls.push(Control::new(
                    KeyCode::Char('C'),
                    "Cancel run",
                    Action::CancelRun,
                ));
                controls.push(Control::new(KeyCode::Char('b'), "Back", Action::Back));
                controls.push(Control::hidden(KeyCode::Left, Action::Back));
                controls.push(Control::hidden(KeyCode::Esc, Action::Back));
            } else {
                controls.push(Control::new(KeyCode::Enter, "Open run", Action::Open));
            }
            controls.push(Control::new(
                KeyCode::Char('w'),
                "Web",
                Action::OpenInBrowser,
            ));
        } else if self.view == View::Local {
            if self.local.commit.is_some() {
                controls.push(Control::new(KeyCode::Char('b'), "Back", Action::Back));
                controls.push(Control::hidden(KeyCode::Left, Action::Back));
                controls.push(Control::hidden(KeyCode::Esc, Action::Back));
            } else if self.local.tab == LocalTab::History {
                controls.push(Control::new(KeyCode::Enter, "Show commit", Action::Open));
            } else {
                if self.local.tab == LocalTab::Working {
                    controls.push(Control::new(KeyCode::Char('+'), "Stage", Action::Stage));
                } else {
                    controls.push(Control::new(KeyCode::Char('-'), "Unstage", Action::Unstage));
                }
                controls.push(Control::new(
                    KeyCode::Char('K'),
                    "Commit",
                    Action::StartCommit,
                ));
            }
            controls.extend(
                LocalTab::ALL.iter().map(|(tab, key, _)| {
                    Control::hidden(KeyCode::Char(*key), Action::LocalTab(*tab))
                }),
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
        let mut hits = match (self.view, &mut self.log) {
            (_, Some(log)) => log.draw(frame, body),
            (View::Inbox, None) => match &mut self.pr {
                Some(pr) => pr.draw(frame, body),
                None => self.inbox.draw(frame, body),
            },
            (View::Local, None) => self.local.draw(frame, body),
            (View::Actions, None) => self.actions.draw(frame, body),
            (View::Notifications, None) => self.notifications.draw(frame, body),
            (View::Search, None) => self.search.draw(frame, body),
            (View::Dashboard, None) => self.dashboard.draw(frame, body),
        };
        let note = self
            .api_note
            .as_deref()
            .map_or(String::new(), |note| format!("  •  {note}"));
        frame.render_widget(Paragraph::new(format!(" {}{note}", self.status)), status);
        hits.extend(keybar::draw(frame, bar, &self.controls()));
        // A picker or the compose box sits over everything; only its own targets answer.
        if let Some(picker) = &self.picker {
            hits = picker.draw(frame, body);
        }
        if let Some(compose) = &self.compose {
            let login = self.login.as_deref().unwrap_or("?");
            hits = compose.draw(frame, body, login);
        }
        self.hits = hits;
    }
}

/// `path:line` of a review thread, for labels.
fn place(thread: &crate::model::Thread) -> String {
    thread.line.map_or(thread.path.clone(), |line| {
        format!("{}:{line}", thread.path)
    })
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
    fn a_cached_inbox_shows_at_once_and_only_for_its_own_account() {
        let mut app = App::default();
        app.restore("octocat".into(), vec![pr(1, "o/a", &[Queue::Authored])], 0);
        let screen = draw(&mut app, 140).join("\n");
        assert!(
            screen.contains("o/a#1") && screen.contains("refreshing"),
            "{screen}"
        );
        app.receive(Message::SignedIn(Ok("octocat".into())));
        assert_eq!(
            app.inbox.visible().len(),
            1,
            "same account keeps the cached list"
        );
        app.receive(Message::Inbox(Ok(vec![])));
        assert!(!app.inbox.open.cached);

        let mut other = App::default();
        other.restore("octocat".into(), vec![pr(1, "o/a", &[])], 0);
        other.receive(Message::SignedIn(Ok("hubot".into())));
        assert!(
            other.inbox.open.prs.is_none(),
            "another account's cache is dropped"
        );
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
    fn a_pr_opens_by_enter_or_click_loads_its_diff_on_files_and_goes_back() {
        for by_click in [false, true] {
            let mut app = signed_in();
            draw(&mut app, 140);
            if by_click {
                click(&mut app, &Action::Select(0)); // already selected: opens
            } else {
                press(&mut app, KeyCode::Enter);
            }
            assert_eq!(
                app.jobs,
                vec![Job::LoadPr {
                    repo: "o/a".into(),
                    number: 1,
                    updated_at: Some("2026-10-27T11:00:00Z".into())
                }]
            );
            app.jobs.clear();
            app.receive(Message::Pr(Ok(Box::new(crate::pr::tests::page()))));
            let screen = draw(&mut app, 140).join("\n");
            assert!(screen.contains("feature → main"), "{screen}");
            if by_click {
                click(&mut app, &Action::PrTab(PrTab::Files));
            } else {
                press(&mut app, KeyCode::Char('f'));
            }
            assert!(matches!(app.jobs.as_slice(), [Job::LoadDiff { .. }]));
            app.receive(Message::Diff {
                base: "b".repeat(40),
                head: "a".repeat(40),
                result: Ok("+added line".into()),
            });
            assert!(draw(&mut app, 140).join("\n").contains("+added line"));
            if by_click {
                click(&mut app, &Action::Back);
            } else {
                press(&mut app, KeyCode::Left);
            }
            assert!(app.pr.is_none());
        }
    }

    #[test]
    fn a_write_previews_its_exact_effect_and_sends_only_on_confirm() {
        for by_click in [false, true] {
            let mut app = signed_in();
            press(&mut app, KeyCode::Enter);
            app.receive(Message::Pr(Ok(Box::new(crate::pr::tests::page()))));
            app.jobs.clear();
            draw(&mut app, 140);
            if by_click {
                click(&mut app, &Action::Compose(WriteKind::RequestChanges));
            } else {
                press(&mut app, KeyCode::Char('X'));
            }
            app.paste("Please split\nthis PR");
            draw(&mut app, 140);
            if by_click {
                click(&mut app, &Action::ComposePreview);
            } else {
                app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
            }
            let screen = draw(&mut app, 140).join("\n");
            assert!(screen.contains("Request changes on o/a#1"), "{screen}");
            assert!(screen.contains("As @octocat"), "{screen}");
            assert!(app.jobs.is_empty(), "nothing is sent by the preview");
            if by_click {
                click(&mut app, &Action::ComposeConfirm);
            } else {
                press(&mut app, KeyCode::Enter);
            }
            assert_eq!(
                app.jobs,
                vec![Job::Write {
                    kind: WriteKind::RequestChanges,
                    pr_id: "PR_1".into(),
                    head_oid: "a".repeat(40),
                    body: "Please split\nthis PR".into(),
                }]
            );
            app.jobs.clear();
            // A failure keeps the text; a success closes the box and reloads the PR.
            app.receive(Message::Written(Err("GitHub: no".into())));
            let compose = app.compose.as_ref().unwrap();
            assert_eq!(compose.editor.text(), "Please split\nthis PR");
            assert_eq!(compose.error.as_deref(), Some("GitHub: no"));
            app.receive(Message::Written(Ok(
                "https://github.com/o/a/pull/1#r".into()
            )));
            assert!(app.compose.is_none());
            assert!(matches!(
                app.jobs.as_slice(),
                [Job::LoadPr {
                    updated_at: None,
                    ..
                }]
            ));
        }
    }

    /// An app with PR o/a#1 open on `tab`, its page and (for Files) a small diff loaded.
    fn open_pr(tab: PrTab) -> App {
        let mut app = signed_in();
        press(&mut app, KeyCode::Enter);
        app.receive(Message::Pr(Ok(Box::new(crate::pr::tests::page()))));
        app.dispatch(Action::PrTab(tab));
        app.receive(Message::Diff {
            base: "b".repeat(40),
            head: "a".repeat(40),
            result: Ok(
                "diff --git a/src/x.rs b/src/x.rs\n@@ -1,2 +1,2 @@\n keep\n-old\n+new".into(),
            ),
        });
        app.jobs.clear();
        app
    }

    #[test]
    fn a_diff_line_takes_a_comment_by_key_or_by_clicking_it_twice() {
        for by_click in [false, true] {
            let mut app = open_pr(PrTab::Files);
            draw(&mut app, 140);
            if by_click {
                click(&mut app, &Action::Cursor(2));
                draw(&mut app, 140);
                click(&mut app, &Action::Cursor(2)); // the line under the cursor
            } else {
                press(&mut app, KeyCode::Down);
                press(&mut app, KeyCode::Down);
                press(&mut app, KeyCode::Char('i'));
            }
            let compose = app.compose.as_ref().expect("compose box open");
            assert_eq!(
                compose.kind,
                WriteKind::LineComment {
                    path: "src/x.rs".into(),
                    line: 2,
                    side: crate::model::Side::Right
                }
            );
            app.paste("Rename this");
            app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
            let screen = draw(&mut app, 140).join("\n");
            assert!(
                screen.contains("Comment on src/x.rs line 2 (new side) of o/a#1"),
                "{screen}"
            );
        }
    }

    #[test]
    fn threads_take_replies_and_resolve_from_the_conversation_tab() {
        let mut app = open_pr(PrTab::Conversation);
        let bar = draw(&mut app, 140).join("\n");
        assert!(
            bar.contains("R Reply") && bar.contains("Z Resolve"),
            "{bar}"
        );
        press(&mut app, KeyCode::Char('R'));
        assert!(matches!(
            &app.compose.as_ref().unwrap().kind,
            WriteKind::Reply { thread_id, place } if thread_id == "T_1" && place == "src/x.rs:7"
        ));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('Z'));
        let compose = app.compose.as_ref().unwrap();
        assert_eq!(
            compose.stage,
            Stage::Preview,
            "resolving has no text to write"
        );
        let screen = draw(&mut app, 140).join("\n");
        assert!(
            screen.contains("Resolve the review thread on src/x.rs:7 of o/a#1"),
            "{screen}"
        );
        press(&mut app, KeyCode::Enter);
        assert!(matches!(
            &app.jobs[..],
            [Job::Write { kind: WriteKind::Resolve { resolve: true, .. }, body, .. }] if body.is_empty()
        ));
    }

    #[test]
    fn a_merge_previews_method_base_and_head_and_refuses_drafts() {
        for by_click in [false, true] {
            let mut app = open_pr(PrTab::Summary);
            draw(&mut app, 140);
            if by_click {
                click(&mut app, &Action::Merge);
            } else {
                press(&mut app, KeyCode::Char('M'));
            }
            let screen = draw(&mut app, 140).join("\n");
            assert!(
                screen.contains("Merge o/a#1 into main by merge commit"),
                "{screen}"
            );
            assert!(screen.contains("GitHub merge state: clean"), "{screen}");
            assert!(
                !screen.contains("r Rebase"),
                "only the repository's methods: {screen}"
            );
            if by_click {
                click(&mut app, &Action::MergeMethod(MergeMethod::Squash));
            } else {
                press(&mut app, KeyCode::Char('s'));
            }
            press(&mut app, KeyCode::Char('r')); // not allowed here: ignored
            assert!(draw(&mut app, 140).join("\n").contains("by squash"));
            press(&mut app, KeyCode::Enter);
            assert!(matches!(
                &app.jobs[..],
                [Job::Write { kind: WriteKind::Merge { method: MergeMethod::Squash }, head_oid, .. }]
                    if *head_oid == "a".repeat(40)
            ));
            app.jobs.clear();
            app.receive(Message::Written(Ok("https://github.com/o/a/pull/1".into())));
            assert!(
                app.jobs.contains(&Job::LoadInbox),
                "the Inbox reloads after a merge"
            );
        }
        let mut draft = open_pr(PrTab::Summary);
        draft.pr.as_mut().unwrap().page.as_mut().unwrap().draft = true;
        press(&mut draft, KeyCode::Char('M'));
        assert!(draft.compose.is_none());
        assert!(draft.status.contains("draft"), "{}", draft.status);
    }

    #[test]
    fn a_check_opens_its_log_at_the_first_failure_by_key_or_click() {
        for by_click in [false, true] {
            let mut app = open_pr(PrTab::Checks);
            let screen = draw(&mut app, 140).join("\n");
            assert!(screen.contains("Enter Log"), "{screen}");
            if by_click {
                click(&mut app, &Action::Cursor(0)); // already under the cursor: opens
            } else {
                press(&mut app, KeyCode::Enter);
            }
            assert_eq!(
                app.jobs,
                vec![Job::LoadLog {
                    repo: "o/a".into(),
                    job_id: 42,
                    finished: true
                }]
            );
            app.receive(Message::Log {
                repo: "o/a".into(),
                job_id: 42,
                finished: true,
                result: Ok("ok\nerror: boom\nmore\nerror: again".into()),
            });
            let screen = draw(&mut app, 140).join("\n");
            assert!(screen.contains("failure 1 of 2"), "{screen}");
            if by_click {
                click(&mut app, &Action::Failure(1));
            } else {
                press(&mut app, KeyCode::Char('>'));
            }
            assert!(draw(&mut app, 140).join("\n").contains("failure 2 of 2"));
            press(&mut app, KeyCode::Char('b'));
            assert!(
                app.log.is_none() && app.pr.is_some(),
                "Back closes only the log"
            );
        }
    }

    #[test]
    fn actions_runs_open_to_jobs_logs_and_a_previewed_rerun() {
        use crate::actions::tests::{job, run};
        for by_click in [false, true] {
            let mut app = App {
                actions: ActionsView::new(Ok("o/a".into())),
                jobs: Vec::new(),
                ..App::default()
            };
            press(&mut app, KeyCode::Char('3'));
            assert_eq!(
                app.jobs,
                vec![Job::LoadRuns {
                    repo: "o/a".into(),
                    saved: None
                }]
            );
            app.jobs.clear();
            app.receive(Message::Runs {
                repo: "o/a".into(),
                result: Ok((vec![run(1, Some("success")), run(2, Some("failure"))], None)),
            });
            let screen = draw(&mut app, 140).join("\n");
            assert!(
                screen.contains("#2 Change 2") && screen.contains("✗"),
                "{screen}"
            );
            if by_click {
                click(&mut app, &Action::Select(1));
                click(&mut app, &Action::Select(1)); // the selected run opens
            } else {
                press(&mut app, KeyCode::Down);
                press(&mut app, KeyCode::Enter);
            }
            assert!(matches!(&app.jobs[..], [Job::LoadJobs { run_id: 2, .. }]));
            app.jobs.clear();
            app.receive(Message::Jobs {
                repo: "o/a".into(),
                run_id: 2,
                result: Ok((vec![job(10, "success"), job(11, "failure")], None)),
            });
            let screen = draw(&mut app, 140).join("\n");
            assert!(
                screen.contains("job 11") && screen.contains("1. cargo test"),
                "{screen}"
            );
            press(&mut app, KeyCode::Down);
            press(&mut app, KeyCode::Enter);
            assert!(matches!(
                &app.jobs[..],
                [Job::LoadLog {
                    job_id: 11,
                    finished: true,
                    ..
                }]
            ));
            app.jobs.clear();
            press(&mut app, KeyCode::Char('b')); // closes the log
            assert!(app.log.is_none() && app.actions.open.is_some());
            draw(&mut app, 140);
            if by_click {
                click(&mut app, &Action::Rerun);
            } else {
                press(&mut app, KeyCode::Char('R'));
            }
            let screen = draw(&mut app, 140).join("\n");
            assert!(
                screen.contains("Re-run the failed jobs of CI #2 in o/a"),
                "{screen}"
            );
            assert!(app.jobs.is_empty(), "nothing is sent by the preview");
            press(&mut app, KeyCode::Enter);
            assert!(matches!(
                &app.jobs[..],
                [Job::Write {
                    kind: WriteKind::Rerun { run_id: 2, .. },
                    ..
                }]
            ));
        }
    }

    #[test]
    fn files_stage_by_key_or_click_and_commit_through_a_preview() {
        use crate::git::Status;
        for by_click in [false, true] {
            let repo = Repo {
                root: "/tmp/repo".into(),
            };
            let mut app = App {
                local: Local::new(Ok(repo.clone())),
                jobs: Vec::new(),
                ..App::default()
            };
            press(&mut app, KeyCode::Char('2'));
            press(&mut app, KeyCode::Char('w'));
            app.jobs.clear();
            app.receive(Message::Changes(Ok(Changes {
                status: Status {
                    branch: "main".into(),
                    staged: vec![("done.rs".into(), 'M')],
                    unstaged: vec![("a.rs".into(), 'M'), ("b.rs".into(), '?')],
                    ..Status::default()
                },
                working: "diff --git a/a.rs b/a.rs\n+edit".into(),
                staged: String::new(),
                author: "Ada <ada@example.invalid>".into(),
            })));
            let screen = draw(&mut app, 120).join("\n");
            assert!(
                screen.contains("+edit") && screen.contains("+ Stage"),
                "{screen}"
            );
            if by_click {
                click(&mut app, &Action::Select(1));
            } else {
                press(&mut app, KeyCode::Down);
            }
            assert!(draw(&mut app, 120).join("\n").contains("New file b.rs"));
            if by_click {
                click(&mut app, &Action::Stage);
            } else {
                press(&mut app, KeyCode::Char('+'));
            }
            assert_eq!(
                app.jobs,
                vec![Job::GitWrite(repo.clone(), GitOp::Stage("b.rs".into()))]
            );
            app.jobs.clear();
            press(&mut app, KeyCode::Char('K'));
            app.paste("Add b");
            app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
            let screen = draw(&mut app, 120).join("\n");
            assert!(
                screen.contains("Commit 1 staged file(s) on main"),
                "{screen}"
            );
            assert!(screen.contains("As Ada <ada@example.invalid>"), "{screen}");
            assert!(
                screen.contains("M done.rs") && screen.contains("│ Add b"),
                "{screen}"
            );
            press(&mut app, KeyCode::Enter);
            assert_eq!(
                app.jobs,
                vec![Job::GitWrite(repo.clone(), GitOp::Commit("Add b".into()))]
            );
            app.jobs.clear();
            app.receive(Message::GitDone {
                commit: true,
                result: Ok("0123456789abcdef".into()),
            });
            assert!(app.compose.is_none());
            assert_eq!(app.status, "Committed 01234567");
            assert_eq!(app.jobs.len(), 2, "history and changes reload");
        }
    }

    #[test]
    fn commits_and_timeline_load_once_when_their_tab_opens() {
        for by_click in [false, true] {
            let mut app = open_pr(PrTab::Summary);
            draw(&mut app, 140);
            if by_click {
                click(&mut app, &Action::PrTab(PrTab::Commits));
            } else {
                press(&mut app, KeyCode::Char('h'));
            }
            assert_eq!(
                app.jobs,
                vec![Job::LoadPrCommits {
                    repo: "o/a".into(),
                    number: 1
                }]
            );
            app.jobs.clear();
            app.receive(Message::PrCommits {
                number: 1,
                result: Ok(vec![PrCommit {
                    oid: "c".repeat(40),
                    headline: "First change".into(),
                    author: "alice".into(),
                    date: "2026-10-27T10:00:00Z".into(),
                    checks: crate::model::Checks::Passing,
                }]),
            });
            let screen = draw(&mut app, 140).join("\n");
            assert!(screen.contains("✓ cccccccc First change"), "{screen}");
            press(&mut app, KeyCode::Char('t'));
            app.receive(Message::PrTimeline {
                number: 1,
                result: Ok(vec![Event {
                    at: "2026-10-27T10:00:00Z".into(),
                    actor: "bob".into(),
                    text: "added the label bug".into(),
                }]),
            });
            assert!(
                draw(&mut app, 140)
                    .join("\n")
                    .contains("bob added the label bug")
            );
            app.jobs.clear();
            press(&mut app, KeyCode::Char('h'));
            assert!(app.jobs.is_empty(), "the commits are not read again");
        }
    }

    #[test]
    fn typing_in_the_compose_box_never_triggers_view_keys() {
        let mut app = signed_in();
        press(&mut app, KeyCode::Enter);
        app.receive(Message::Pr(Ok(Box::new(crate::pr::tests::page()))));
        press(&mut app, KeyCode::Char('n'));
        for c in "qbw".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert!(!app.quit && app.pr.is_some());
        assert_eq!(app.compose.as_ref().unwrap().editor.text(), "qbw");
        press(&mut app, KeyCode::Esc);
        assert!(app.compose.is_none(), "Esc while editing cancels");
    }

    #[test]
    fn local_history_loads_on_first_visit_and_shows_a_commit_by_key_or_click() {
        for by_click in [false, true] {
            let mut app = App {
                local: Local::new(Ok(Repo {
                    root: "/tmp/repo".into(),
                })),
                jobs: Vec::new(),
                ..App::default()
            };
            press(&mut app, KeyCode::Char('2'));
            assert_eq!(app.jobs.len(), 2, "history and changes load together");
            app.jobs.clear();
            app.receive(Message::History(Ok(crate::local::tests::history())));
            let screen = draw(&mut app, 120).join("\n");
            assert!(
                screen.contains("Second") && screen.contains("aaaaaaaa"),
                "{screen}"
            );
            if by_click {
                click(&mut app, &Action::Select(2));
                click(&mut app, &Action::Select(2)); // the selected row opens
            } else {
                press(&mut app, KeyCode::Down);
                press(&mut app, KeyCode::Enter);
            }
            let sha = "b".repeat(40);
            assert!(matches!(&app.jobs[..], [Job::ShowCommit(_, s)] if *s == sha));
            app.receive(Message::Commit {
                sha: sha.clone(),
                result: Ok("commit bbbb\n+line".into()),
            });
            assert!(draw(&mut app, 120).join("\n").contains("+line"));
            press(&mut app, KeyCode::Char('b'));
            assert!(app.local.commit.is_none());
            press(&mut app, KeyCode::Char('i'));
            assert_eq!(app.local.tab, LocalTab::Staged);
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

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    /// The picker row showing `label`, as drawn.
    fn picker_row(app: &App, label: &str) -> usize {
        let picker = app.picker.as_ref().unwrap();
        let items = picker.items.as_ref().unwrap().as_ref().unwrap();
        picker
            .shown()
            .iter()
            .position(|&index| items[index].label == label)
            .unwrap()
    }

    #[test]
    fn the_pr_menu_closes_a_pr_by_key_or_by_click() {
        for by_click in [false, true] {
            let mut app = open_pr(PrTab::Summary);
            draw(&mut app, 140);
            if by_click {
                click(&mut app, &Action::PrMenu);
            } else {
                press(&mut app, KeyCode::Char('p'));
            }
            let screen = draw(&mut app, 140).join("\n");
            assert!(screen.contains("PR actions"), "{screen}");
            assert!(screen.contains("Convert to draft"), "{screen}");
            assert!(!screen.contains("Reopen"), "an open PR offers Close only");
            if by_click {
                let row = picker_row(&app, "Close");
                click(&mut app, &Action::PickerRow(row));
            } else {
                type_text(&mut app, "clo");
                press(&mut app, KeyCode::Enter);
            }
            assert!(app.picker.is_none());
            let screen = draw(&mut app, 140).join("\n");
            assert!(screen.contains("Close o/a#1"), "{screen}");
            assert!(app.jobs.is_empty(), "nothing is sent before confirming");
            press(&mut app, KeyCode::Enter);
            assert!(matches!(
                &app.jobs[..],
                [Job::Write { kind: WriteKind::Close, pr_id, .. }] if pr_id == "PR_1"
            ));
            app.jobs.clear();
            app.receive(Message::Written(Ok("https://github.com/o/a/pull/1".into())));
            assert!(app.jobs.contains(&Job::LoadInbox), "{:?}", app.jobs);
        }
    }

    #[test]
    fn labels_are_checked_in_a_picker_and_the_preview_shows_the_change() {
        let mut app = open_pr(PrTab::Summary);
        press(&mut app, KeyCode::Char('p'));
        type_text(&mut app, "label");
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.jobs,
            vec![Job::LoadChoices {
                repo: "o/a".into(),
                purpose: Purpose::Labels
            }]
        );
        assert!(draw(&mut app, 140).join("\n").contains("Loading…"));
        app.receive(Message::Choices {
            purpose: Purpose::Labels,
            result: Ok(vec![Item::new("L_bug", "bug"), Item::new("L_wip", "wip")]),
        });
        let screen = draw(&mut app, 140).join("\n");
        assert!(screen.contains("[ ] bug"), "{screen}");
        assert!(
            screen.contains("[x] wip"),
            "the PR's own label starts checked"
        );
        // Space toggles by key; clicking a row toggles it too.
        press(&mut app, KeyCode::Char(' '));
        let row = picker_row(&app, "wip");
        click(&mut app, &Action::PickerRow(row));
        press(&mut app, KeyCode::Enter);
        let compose = app.compose.as_ref().unwrap();
        assert_eq!(compose.stage, Stage::Preview);
        let screen = draw(&mut app, 140).join("\n");
        assert!(screen.contains("Change the labels of o/a#1"), "{screen}");
        assert!(
            screen.contains("+ bug") && screen.contains("− wip"),
            "{screen}"
        );
        app.jobs.clear();
        press(&mut app, KeyCode::Enter);
        assert!(matches!(
            &app.jobs[..],
            [Job::Write { kind: WriteKind::Labels { add, remove }, .. }]
                if add[0].id == "L_bug" && remove[0].id == "L_wip"
        ));
    }

    #[test]
    fn reviewers_need_a_change_and_go_to_the_prs_repository() {
        let mut app = open_pr(PrTab::Summary);
        press(&mut app, KeyCode::Char('p'));
        type_text(&mut app, "review");
        press(&mut app, KeyCode::Enter);
        app.receive(Message::Choices {
            purpose: Purpose::Reviewers,
            result: Ok(vec![Item::new("alice", "alice"), Item::new("bob", "bob")]),
        });
        // bob is already requested; Done with nothing changed sends nothing.
        press(&mut app, KeyCode::Enter);
        assert!(app.compose.is_none() && app.picker.is_none());
        assert_eq!(app.status, "The review requests are unchanged");
        press(&mut app, KeyCode::Char('p'));
        type_text(&mut app, "review");
        press(&mut app, KeyCode::Enter);
        app.receive(Message::Choices {
            purpose: Purpose::Reviewers,
            result: Ok(vec![Item::new("alice", "alice"), Item::new("bob", "bob")]),
        });
        type_text(&mut app, "ali");
        press(&mut app, KeyCode::Char(' '));
        draw(&mut app, 140);
        click(&mut app, &Action::PickerDone);
        assert!(draw(&mut app, 140).join("\n").contains("+ @alice"));
        app.jobs.clear();
        press(&mut app, KeyCode::Enter);
        assert!(matches!(
            &app.jobs[..],
            [Job::Write { kind: WriteKind::Reviewers { repo, number: 1, add, remove }, .. }]
                if repo == "o/a" && add == &["alice"] && remove.is_empty()
        ));
    }

    #[test]
    fn a_rename_starts_from_the_title_and_previews_both() {
        let mut app = open_pr(PrTab::Summary);
        press(&mut app, KeyCode::Char('p'));
        type_text(&mut app, "title");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.compose.as_ref().unwrap().editor.text(), "Change 1");
        type_text(&mut app, "!");
        app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        let screen = draw(&mut app, 140).join("\n");
        assert!(screen.contains("From: Change 1"), "{screen}");
        assert!(screen.contains("To:   Change 1!"), "{screen}");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(
            &app.jobs[..],
            [Job::Write { kind: WriteKind::Title { from }, body, .. }]
                if from == "Change 1" && body == "Change 1!"
        ));
    }

    #[test]
    fn esc_closes_the_picker_and_a_draft_offers_ready_for_review() {
        let mut app = open_pr(PrTab::Summary);
        app.pr.as_mut().unwrap().page.as_mut().unwrap().draft = true;
        press(&mut app, KeyCode::Char('p'));
        let screen = draw(&mut app, 140).join("\n");
        assert!(screen.contains("Ready for review"), "{screen}");
        press(&mut app, KeyCode::Esc);
        assert!(app.picker.is_none());
        assert!(app.pr.is_some(), "Esc closes only the picker");
    }

    fn with_notifications() -> App {
        use crate::notifications::tests::notification;
        let mut app = signed_in();
        press(&mut app, KeyCode::Char('4'));
        assert_eq!(
            app.jobs,
            vec![Job::LoadNotifications {
                all: false,
                saved: None
            }]
        );
        app.receive(Message::Notifications {
            all: false,
            result: Ok((
                vec![
                    notification("t1", "PullRequest", 7),
                    notification("t2", "Issue", 8),
                ],
                None,
            )),
        });
        app.jobs.clear();
        app
    }

    #[test]
    fn a_pr_notification_opens_the_pr_and_back_returns_to_notifications() {
        for by_click in [false, true] {
            let mut app = with_notifications();
            let screen = draw(&mut app, 140).join("\n");
            assert!(screen.contains("Notifications · unread"), "{screen}");
            assert!(screen.contains("Subject t1") && screen.contains("review requested"));
            if by_click {
                click(&mut app, &Action::Select(0)); // the selected row opens
            } else {
                press(&mut app, KeyCode::Enter);
            }
            assert_eq!(app.view, View::Inbox);
            assert!(matches!(
                &app.jobs[..],
                [Job::LoadPr { repo, number: 7, updated_at: None }] if repo == "o/a"
            ));
            press(&mut app, KeyCode::Esc);
            assert!(app.pr.is_none());
            assert_eq!(app.view, View::Notifications);
            press(&mut app, KeyCode::Esc);
            assert_eq!(app.view, View::Notifications, "a second Back stays put");
        }
    }

    #[test]
    fn other_notifications_open_on_the_web() {
        let mut app = with_notifications();
        draw(&mut app, 140);
        click(&mut app, &Action::Select(1));
        assert!(app.jobs.is_empty(), "the first click selects");
        click(&mut app, &Action::Select(1));
        assert_eq!(
            app.jobs,
            vec![Job::Open("https://github.com/o/a/issues/8".into())]
        );
    }

    #[test]
    fn marking_read_or_done_previews_then_reloads() {
        for (key, action) in [('x', Action::MarkRead), ('D', Action::MarkDone)] {
            for by_click in [false, true] {
                let mut app = with_notifications();
                draw(&mut app, 140);
                if by_click {
                    click(&mut app, &action);
                } else {
                    press(&mut app, KeyCode::Char(key));
                }
                let screen = draw(&mut app, 140).join("\n");
                assert!(screen.contains("the notification in o/a"), "{screen}");
                assert!(screen.contains("Subject t1"), "{screen}");
                assert!(app.jobs.is_empty(), "nothing is sent before confirming");
                press(&mut app, KeyCode::Enter);
                let thread = match &app.jobs[..] {
                    [
                        Job::Write {
                            kind:
                                WriteKind::MarkRead { thread_id } | WriteKind::MarkDone { thread_id },
                            ..
                        },
                    ] => thread_id.clone(),
                    other => panic!("{other:?}"),
                };
                assert_eq!(thread, "t1");
                app.jobs.clear();
                app.receive(Message::Written(Ok("marked".into())));
                assert!(
                    app.jobs.contains(&Job::LoadNotifications {
                        all: false,
                        saved: None
                    }),
                    "{:?}",
                    app.jobs
                );
            }
        }
    }

    #[test]
    fn all_and_unread_switch_by_key_and_click() {
        let mut app = with_notifications();
        draw(&mut app, 140);
        click(&mut app, &Action::NotificationsAll(true));
        assert!(app.notifications.all && app.notifications.items.is_none());
        assert_eq!(
            app.jobs,
            vec![Job::LoadNotifications {
                all: true,
                saved: None
            }]
        );
        // A late answer for the other list is ignored.
        app.receive(Message::Notifications {
            all: false,
            result: Ok((Vec::new(), None)),
        });
        assert!(app.notifications.items.is_none());
        app.jobs.clear();
        press(&mut app, KeyCode::Char('u'));
        assert!(!app.notifications.all);
        assert!(
            draw(&mut app, 140)
                .join("\n")
                .contains("Loading notifications")
        );
    }

    fn hit(repo: &str, number: i64, pr: bool) -> Hit {
        Hit {
            repo: repo.into(),
            number,
            title: format!("Found {number}"),
            url: format!("https://github.com/{repo}/issues/{number}"),
            pr,
            state: "open".into(),
            author: "alice".into(),
            updated_at: "2026-10-27T10:00:00Z".into(),
        }
    }

    fn searched() -> App {
        let mut app = signed_in();
        press(&mut app, KeyCode::Char('5'));
        assert!(app.search.editing, "Search starts in its query");
        type_text(&mut app, "is:pr q");
        assert_eq!(
            app.view,
            View::Search,
            "digits and letters type, not switch"
        );
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.jobs, vec![Job::Search("is:pr".into())]);
        assert!(draw(&mut app, 140).join("\n").contains("Searching…"));
        app.receive(Message::Search {
            query: "is:pr".into(),
            result: Ok((
                vec![
                    hit("o/a", 3, true),
                    hit("o/hidden", 4, true),
                    hit("o/a", 5, false),
                ],
                120,
            )),
        });
        app.jobs.clear();
        app
    }

    #[test]
    fn a_search_runs_on_enter_and_hides_hidden_repositories() {
        let mut app = App::default();
        app.inbox.hidden = vec!["o/hidden".into()];
        app.receive(Message::SignedIn(Ok("octocat".into())));
        app.jobs.clear();
        press(&mut app, KeyCode::Char('5'));
        press(&mut app, KeyCode::Enter);
        assert!(app.jobs.is_empty(), "an empty search is not sent");
        type_text(&mut app, "is:pr");
        press(&mut app, KeyCode::Enter);
        app.receive(Message::Search {
            query: "is:pr".into(),
            result: Ok((vec![hit("o/a", 3, true), hit("o/hidden", 4, true)], 2)),
        });
        let screen = draw(&mut app, 140).join("\n");
        assert!(
            screen.contains("o/a#3") && !screen.contains("o/hidden"),
            "{screen}"
        );
        assert!(screen.contains("is:pr · 1 of 2"), "{screen}");
    }

    #[test]
    fn a_found_pr_opens_by_key_or_click_and_back_returns_to_search() {
        for by_click in [false, true] {
            let mut app = searched();
            draw(&mut app, 140);
            if by_click {
                click(&mut app, &Action::Select(0)); // the selected row opens
            } else {
                press(&mut app, KeyCode::Enter);
            }
            assert!(matches!(
                &app.jobs[..],
                [Job::LoadPr { repo, number: 3, .. }] if repo == "o/a"
            ));
            press(&mut app, KeyCode::Left);
            assert_eq!(app.view, View::Search);
            assert_eq!(app.search.ran.as_deref(), Some("is:pr"), "the results stay");
        }
    }

    #[test]
    fn an_issue_opens_on_the_web_and_slash_edits_the_query() {
        let mut app = searched();
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.jobs,
            vec![Job::Open("https://github.com/o/a/issues/5".into())]
        );
        app.jobs.clear();
        press(&mut app, KeyCode::Char('/'));
        assert!(app.search.editing);
        type_text(&mut app, "is:open");
        draw(&mut app, 140);
        click(&mut app, &Action::RunSearch);
        assert_eq!(app.jobs, vec![Job::Search("is:pr is:open".into())]);
    }

    #[test]
    fn refresh_reruns_only_the_last_search() {
        let mut app = searched();
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, " half");
        app.dispatch(Action::Refresh);
        assert!(app.jobs.is_empty(), "nothing runs while typing");
        press(&mut app, KeyCode::Esc);
        app.dispatch(Action::Refresh);
        assert_eq!(app.jobs, vec![Job::Search("is:pr".into())]);
        // A late answer to an older query is ignored.
        app.receive(Message::Search {
            query: "older".into(),
            result: Ok((Vec::new(), 0)),
        });
        assert_eq!(app.search.selected_hit().unwrap().number, 3);
    }

    fn dashboard() -> App {
        let mut app = signed_in();
        app.dashboard = DashboardView::new(Ok("o/a".into()));
        press(&mut app, KeyCode::Char('6'));
        assert_eq!(
            app.jobs,
            vec![Job::LoadDashboard {
                repo: "o/a".into(),
                days: 7
            }]
        );
        app.receive(Message::Dashboard {
            repo: "o/a".into(),
            days: 7,
            result: Ok(Box::new(crate::dashboard::tests::metrics())),
        });
        app.jobs.clear();
        app
    }

    #[test]
    fn dashboard_cards_open_their_records_and_a_pr_returns_to_them() {
        for by_click in [false, true] {
            let mut app = dashboard();
            let screen = draw(&mut app, 140).join("\n");
            assert!(screen.contains("o/a · last 7 days"), "{screen}");
            assert!(screen.contains("PRs merged") && screen.contains("250 (list shows 100)"));
            if by_click {
                click(&mut app, &Action::Select(1));
                draw(&mut app, 140);
                click(&mut app, &Action::Select(1)); // the selected card opens
            } else {
                press(&mut app, KeyCode::Down);
                press(&mut app, KeyCode::Enter);
            }
            assert_eq!(app.dashboard.open, Some(1));
            let screen = draw(&mut app, 140).join("\n");
            assert!(
                screen.contains("PR 3") && screen.contains("took 1d"),
                "{screen}"
            );
            press(&mut app, KeyCode::Enter);
            assert!(matches!(
                &app.jobs[..],
                [Job::LoadPr { repo, number: 2, .. }] if repo == "o/a"
            ));
            press(&mut app, KeyCode::Esc);
            assert_eq!(app.view, View::Dashboard);
            assert_eq!(app.dashboard.open, Some(1), "the card's records still show");
            press(&mut app, KeyCode::Esc);
            assert_eq!(app.dashboard.open, None);
        }
    }

    #[test]
    fn a_failed_run_opens_on_the_web_and_d_switches_the_window() {
        let mut app = dashboard();
        app.dashboard.select(5); // Failed runs
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.jobs,
            vec![Job::Open("https://github.com/o/a/actions/runs/2".into())]
        );
        app.jobs.clear();
        draw(&mut app, 140);
        click(&mut app, &Action::DashboardDays(30));
        assert_eq!(
            app.jobs,
            vec![Job::LoadDashboard {
                repo: "o/a".into(),
                days: 30
            }]
        );
        // A late answer for the old window is ignored.
        app.receive(Message::Dashboard {
            repo: "o/a".into(),
            days: 7,
            result: Ok(Box::new(crate::dashboard::tests::metrics())),
        });
        assert!(app.dashboard.metrics.is_none());
        assert!(draw(&mut app, 140).join("\n").contains("last 30 days"));
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.dashboard.days, 7);
    }

    #[test]
    fn a_failed_refresh_keeps_the_numbers() {
        let mut app = dashboard();
        press(&mut app, KeyCode::Char('r'));
        app.receive(Message::Dashboard {
            repo: "o/a".into(),
            days: 7,
            result: Err("GitHub returned HTTP 502".into()),
        });
        assert!(matches!(app.dashboard.metrics, Some(Ok(_))));
        assert_eq!(app.status, "GitHub returned HTTP 502");
    }
}
