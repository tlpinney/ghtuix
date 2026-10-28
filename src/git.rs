//! The local checkout through the installed `git`.
//!
//! Reads (`run`) never change it: `--no-optional-locks` (and `GIT_OPTIONAL_LOCKS=0`) stop
//! `git status` from refreshing the index as a side effect; there are no prompts, pagers,
//! external diff tools, textconv filters or fsmonitor hooks, and paths are literal. The
//! few writes (`write`) run only when the user asks for them.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// One line of `git log --graph`: the graph drawing, and the commit on it if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphLine {
    pub graph: String,
    pub commit: Option<Commit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    /// Branch and tag names pointing here, as `git log --decorate` shows them.
    pub refs: String,
}

/// `git status`: the branch and how it stands against its upstream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// `(path, code)` for changes in the index, such as `M` or `A`.
    pub staged: Vec<(String, char)>,
    /// `(path, code)` for changes in the working tree; `?` is untracked.
    pub unstaged: Vec<(String, char)>,
}

/// A local branch, as `git for-each-ref refs/heads` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub current: bool,
    pub upstream: Option<String>,
    /// `ahead 1, behind 2`, `gone`, or empty.
    pub track: String,
    pub date: String,
    pub subject: String,
}

/// A stash entry: `stash@{0}`, when it was made and its message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stash {
    pub name: String,
    pub date: String,
    pub message: String,
    /// The stash's commit: `stash@{N}` moves when stashes are added, this does not.
    pub sha: String,
}

/// Branches, stashes and an operation left in progress (a rebase that stopped).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refs {
    pub branches: Vec<Branch>,
    pub stashes: Vec<Stash>,
    /// `rebase` while a rebase waits to be continued or aborted.
    pub operation: Option<String>,
}

/// What a rebase onto a branch would do: the commits it replays and those it brings in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebasePlan {
    pub replay: Vec<String>,
    pub incoming: usize,
}

/// A local operation on branches, stashes or a rebase. Each runs only after its preview
/// is confirmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalOp {
    Switch(String),
    /// Create a branch at HEAD and switch to it; the name is the compose box's text.
    CreateBranch,
    DeleteBranch(String),
    Rebase {
        onto: String,
        plan: RebasePlan,
    },
    RebaseContinue,
    RebaseAbort,
    /// Stash every change, untracked files too; the message is the compose box's text.
    StashPush,
    StashApply(Stash),
    StashPop(Stash),
    StashDrop(Stash),
}

impl LocalOp {
    /// The git command line the preview shows (`text` is the compose box's text).
    pub fn command(&self, text: &str) -> String {
        let text = text.trim();
        match self {
            Self::Switch(branch) => format!("git switch {branch}"),
            Self::CreateBranch => format!("git switch -c {text}"),
            Self::DeleteBranch(branch) => format!("git branch -d {branch}"),
            Self::Rebase { onto, .. } => format!("git rebase {onto}"),
            Self::RebaseContinue => "git rebase --continue".into(),
            Self::RebaseAbort => "git rebase --abort".into(),
            Self::StashPush if text.is_empty() => "git stash push --include-untracked -- .".into(),
            Self::StashPush => format!("git stash push --include-untracked -m \"{text}\" -- ."),
            Self::StashApply(stash) => format!("git stash apply {}", stash.name),
            Self::StashPop(stash) => format!("git stash pop {}", stash.name),
            Self::StashDrop(stash) => format!("git stash drop {}", stash.name),
        }
    }
}

/// A checkout, found from a directory inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub root: PathBuf,
}

/// Separates the fields of one `git log` record.
const FIELD: char = '\u{1f}';

impl Repo {
    /// The checkout containing `dir`, if it is one.
    pub fn discover(dir: &Path) -> Result<Self> {
        let root = run(dir, &["rev-parse", "--show-toplevel"])?;
        Ok(Self {
            root: PathBuf::from(root.trim_end()),
        })
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        run(&self.root, args)
    }

    /// The newest `limit` commits of every branch, with git's graph.
    pub fn history(&self, limit: usize) -> Result<Vec<GraphLine>> {
        let out = self.git(&[
            "log",
            "--graph",
            "--all",
            "--date-order",
            &format!("--max-count={limit}"),
            "--format=%x1f%H%x1f%an%x1f%aI%x1f%s%x1f%D",
        ])?;
        Ok(out.lines().map(parse_graph_line).collect())
    }

    /// One commit in full: its message, file summary and patch.
    pub fn show(&self, sha: &str) -> Result<String> {
        if !sha.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("Not a commit id: {sha}");
        }
        self.git(&[
            "show",
            "--no-ext-diff",
            "--no-textconv",
            "--stat",
            "--patch",
            "--format=fuller",
            sha,
        ])
    }

    pub fn status(&self) -> Result<Status> {
        let out = self.git(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
        ])?;
        Ok(parse_status(&out))
    }

    /// `owner/name` of the `origin` remote when it is on GitHub host `host`.
    pub fn github_repo(&self, host: &str) -> Option<String> {
        let url = self.git(&["remote", "get-url", "origin"]).ok()?;
        repo_from_url(url.trim(), host)
    }

    /// The author git would record for a commit here (`Name <email>`).
    pub fn author(&self) -> Result<String> {
        let ident = self.git(&["var", "GIT_AUTHOR_IDENT"])?;
        // `Name <email> 1700000000 +0000`: drop the timestamp.
        Ok(ident
            .trim()
            .rsplitn(3, ' ')
            .nth(2)
            .unwrap_or(ident.trim())
            .to_owned())
    }

    /// Stages `path`: a change, a new file or a deletion.
    pub fn stage(&self, path: &str) -> Result<()> {
        write(&self.root, &["add", "-A", "--", path], None).map(|_| ())
    }

    /// Takes `path` out of the index again, keeping the working tree as it is.
    pub fn unstage(&self, path: &str) -> Result<()> {
        if self.git(&["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
            write(&self.root, &["restore", "--staged", "--", path], None).map(|_| ())
        } else {
            // Nothing committed yet: there is no HEAD to restore from.
            write(&self.root, &["rm", "--cached", "-q", "--", path], None).map(|_| ())
        }
    }

    /// Local branches, stashes and any rebase left in progress.
    pub fn refs(&self) -> Result<Refs> {
        let format = [
            "%(HEAD)",
            "%(refname:short)",
            "%(upstream:short)",
            "%(upstream:track,nobracket)",
            "%(committerdate:iso-strict)",
            "%(subject)",
        ]
        .join("%1f");
        let out = self.git(&[
            "for-each-ref",
            &format!("--format={format}"),
            "--sort=-committerdate",
            "refs/heads",
        ])?;
        let branches = out
            .lines()
            .filter_map(|line| {
                let mut fields = line.split(FIELD);
                let mut next = || fields.next().unwrap_or_default().to_owned();
                let (head, name, upstream) = (next(), next(), next());
                Some(Branch {
                    current: head == "*",
                    name: Some(name).filter(|n| !n.is_empty())?,
                    upstream: Some(upstream).filter(|u| !u.is_empty()),
                    track: next(),
                    date: next(),
                    subject: next(),
                })
            })
            .collect();
        let out = self.git(&["stash", "list", "--format=%gd%x1f%cI%x1f%gs%x1f%H"])?;
        let stashes = out
            .lines()
            .filter_map(|line| {
                let mut fields = line.split(FIELD);
                Some(Stash {
                    name: fields.next()?.to_owned(),
                    date: fields.next().unwrap_or_default().to_owned(),
                    message: fields.next().unwrap_or_default().to_owned(),
                    sha: fields.next().unwrap_or_default().to_owned(),
                })
            })
            .collect();
        let rebasing = ["rebase-merge", "rebase-apply"].iter().any(|dir| {
            self.git(&["rev-parse", "--git-path", dir])
                .is_ok_and(|path| self.root.join(path.trim_end()).exists())
        });
        Ok(Refs {
            branches,
            stashes,
            operation: rebasing.then(|| "rebase".to_owned()),
        })
    }

    /// The patch a stash holds (tracked changes; untracked files are listed by name).
    pub fn stash_show(&self, stash: &str) -> Result<String> {
        check_ref(stash)?;
        self.git(&[
            "stash",
            "show",
            "-p",
            "--no-textconv",
            "--include-untracked",
            stash,
        ])
        .or_else(|_| self.git(&["stash", "show", "-p", "--no-textconv", stash]))
    }

    /// What rebasing the current branch onto `onto` would replay and bring in.
    pub fn rebase_plan(&self, onto: &str) -> Result<RebasePlan> {
        check_ref(onto)?;
        let replay = self.git(&[
            "log",
            "--format=%h %s",
            "--reverse",
            &format!("{onto}..HEAD"),
        ])?;
        let incoming = self.git(&["rev-list", "--count", &format!("HEAD..{onto}")])?;
        Ok(RebasePlan {
            replay: replay.lines().map(str::to_owned).collect(),
            incoming: incoming.trim().parse().unwrap_or(0),
        })
    }

    /// Runs a confirmed local operation; returns git's own report of what it did.
    pub fn local_op(&self, op: &LocalOp, text: &str) -> Result<String> {
        let text = text.trim();
        let out = match op {
            LocalOp::Switch(branch) => {
                check_ref(branch)?;
                write(&self.root, &["switch", branch], None)?
            }
            LocalOp::CreateBranch => {
                check_ref(text)?;
                self.git(&["check-ref-format", "--branch", text])
                    .map_err(|_| anyhow::anyhow!("“{text}” is not a valid branch name"))?;
                write(&self.root, &["switch", "-c", text], None)?
            }
            LocalOp::DeleteBranch(branch) => {
                check_ref(branch)?;
                write(&self.root, &["branch", "-d", branch], None)?
            }
            LocalOp::Rebase { onto, .. } => {
                check_ref(onto)?;
                write(&self.root, &["rebase", onto], None)?
            }
            LocalOp::RebaseContinue => write(&self.root, &["rebase", "--continue"], None)?,
            LocalOp::RebaseAbort => write(&self.root, &["rebase", "--abort"], None)?,
            LocalOp::StashPush => {
                // A pathspec is needed: under --literal-pathspecs git otherwise saves
                // untracked files without removing them.
                let mut args = vec!["stash", "push", "--include-untracked"];
                if !text.is_empty() {
                    args.extend(["-m", text]);
                }
                args.extend(["--", "."]);
                write(&self.root, &args, None)?
            }
            LocalOp::StashApply(stash) | LocalOp::StashPop(stash) | LocalOp::StashDrop(stash) => {
                check_ref(&stash.name)?;
                // Refuse if stashing since the list was read moved this name.
                let now = self.git(&["rev-parse", "--verify", "-q", &stash.name]);
                if now.map(|sha| sha.trim().to_owned()).ok().as_deref() != Some(&stash.sha) {
                    bail!("The stash list changed since it was read; press r and choose again");
                }
                let stash = stash.name.as_str();
                let verb = match op {
                    LocalOp::StashApply(_) => "apply",
                    LocalOp::StashPop(_) => "pop",
                    _ => "drop",
                };
                write(&self.root, &["stash", verb, stash], None)?
            }
        };
        Ok(out.trim().to_owned())
    }

    /// Applies `patch` to the index only (the working tree is untouched); `reverse` takes
    /// it out again. Git refuses a patch that no longer fits, as when the file changed.
    pub fn apply_cached(&self, patch: &str, reverse: bool) -> Result<()> {
        let mut args = vec!["apply", "--cached", "--recount"];
        if reverse {
            args.push("--reverse");
        }
        args.push("-");
        write(&self.root, &args, Some(patch)).map(|_| ())
    }

    /// Commits what is staged with `message`; returns the new commit's id.
    pub fn commit(&self, message: &str) -> Result<String> {
        write(&self.root, &["commit", "-q", "-F", "-"], Some(message))?;
        Ok(self.git(&["rev-parse", "HEAD"])?.trim().to_owned())
    }

    /// Changes not yet staged.
    pub fn working_diff(&self) -> Result<String> {
        self.git(&["diff", "--no-ext-diff", "--no-textconv"])
    }

    /// Changes staged for the next commit.
    pub fn staged_diff(&self) -> Result<String> {
        self.git(&["diff", "--no-ext-diff", "--no-textconv", "--cached"])
    }
}

/// Refuses a name git would read as an option (or an empty one).
fn check_ref(name: &str) -> Result<()> {
    if name.is_empty() || name.starts_with('-') {
        bail!("“{name}” is not a branch or stash name");
    }
    Ok(())
}

/// Runs a read-only git command in `dir` and returns its output.
fn run(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(dir)
        .args([
            "--no-optional-locks",
            "--no-pager",
            "--literal-pathspecs",
            // A checkout's own config must not run programs while it is only browsed.
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.quotepath=false",
            "-c",
            "color.ui=never",
        ])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_EXTERNAL_DIFF")
        .env_remove("GIT_PAGER")
        .output()
        .context("Could not run git; is it installed?")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {}: {}", args[0], stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `owner/name` from a remote URL on `host` (`git@host:o/n.git`, `https://host/o/n`,
/// `ssh://git@host/o/n.git`).
fn repo_from_url(url: &str, host: &str) -> Option<String> {
    let path = url
        .strip_prefix(&format!("git@{host}:"))
        .or_else(|| url.strip_prefix(&format!("https://{host}/")))
        .or_else(|| url.strip_prefix(&format!("ssh://git@{host}/")))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let (owner, name) = path.split_once('/')?;
    // Only names GitHub allows, so a remote cannot steer API paths (`..`, `?`, `#`).
    let valid = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (valid(owner) && valid(name)).then(|| format!("{owner}/{name}"))
}

/// Runs a git command that changes the repository: staging, unstaging or committing.
/// These are the only writes ghtuix makes to a checkout, each on the user's request.
fn write(dir: &Path, args: &[&str], stdin: Option<&str>) -> Result<String> {
    use std::io::Write as _;
    let mut child = Command::new("git")
        .current_dir(dir)
        .args(["--no-pager", "--literal-pathspecs", "-c", "color.ui=never"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        // No editor may start (rebase --continue would open one): keep messages as they are.
        .env("GIT_EDITOR", "true")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Could not run git; is it installed?")?;
    if let Some(mut input) = child.stdin.take() {
        input.write_all(stdin.unwrap_or_default().as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let why = if stderr.trim().is_empty() {
            stdout
        } else {
            stderr
        };
        bail!("git {}: {}", args[0], why.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The part of a unified `diff` about `path` (empty when it has none, as for a new file).
pub fn diff_of(diff: &str, path: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            inside = line.trim_end().ends_with(&format!(" b/{path}"));
        } else if let Some(conflicted) = line.strip_prefix("diff --cc ") {
            inside = conflicted.trim_end() == path;
        }
        if inside {
            out.push_str(line);
        }
    }
    out
}

fn parse_graph_line(line: &str) -> GraphLine {
    let Some((graph, record)) = line.split_once(FIELD) else {
        return GraphLine {
            graph: line.to_owned(),
            commit: None,
        };
    };
    let mut fields = record.split(FIELD);
    let mut next = || fields.next().unwrap_or_default().to_owned();
    GraphLine {
        graph: graph.to_owned(),
        commit: Some(Commit {
            sha: next(),
            author: next(),
            date: next(),
            subject: next(),
            refs: next(),
        }),
    }
}

fn parse_status(out: &str) -> Status {
    let mut status = Status::default();
    for line in out.lines() {
        if let Some(head) = line.strip_prefix("# branch.head ") {
            status.branch = head.to_owned();
        } else if let Some(upstream) = line.strip_prefix("# branch.upstream ") {
            status.upstream = Some(upstream.to_owned());
        } else if let Some(counts) = line.strip_prefix("# branch.ab ") {
            for count in counts.split_whitespace() {
                let value = count[1..].parse().unwrap_or(0);
                match count.as_bytes().first() {
                    Some(b'+') => status.ahead = value,
                    Some(b'-') => status.behind = value,
                    _ => {}
                }
            }
        } else if let Some(path) = line.strip_prefix("? ") {
            status.unstaged.push((path.to_owned(), '?'));
        } else if let Some(rest) = line.strip_prefix("u ") {
            // A conflict: `u XY sub m1 m2 m3 mW h1 h2 h3 path`. It is staged (resolved)
            // whole, from the Working list.
            let path = rest.splitn(10, ' ').nth(9).unwrap_or_default();
            status.unstaged.push((path.to_owned(), 'U'));
        } else if let Some((rest, fields)) = line
            .strip_prefix("1 ")
            .map(|rest| (rest, 8))
            .or_else(|| line.strip_prefix("2 ").map(|rest| (rest, 9)))
        {
            // `1 XY sub mH mI mW hH hI path`; a rename (`2`) has a score before the path
            // and its original path after a tab.
            let mut parts = rest.splitn(fields, ' ');
            let codes: Vec<char> = parts.next().unwrap_or("..").chars().collect();
            let path = parts.nth(fields - 2).unwrap_or_default();
            let path = path.split('\t').next().unwrap_or(path).to_owned();
            if codes.first().is_some_and(|c| *c != '.') {
                status.staged.push((path.clone(), codes[0]));
            }
            if codes.get(1).is_some_and(|c| *c != '.') {
                status.unstaged.push((path, codes[1]));
            }
        }
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway repository with one commit, a staged edit and an untracked file.
    fn repo() -> (Repo, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "ghtuix-git-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .current_dir(&dir)
                .args(["-c", "user.name=T", "-c", "user.email=t@example.invalid"])
                .args(args)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "T"]);
        git(&["config", "user.email", "t@example.invalid"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        git(&["add", "a.txt"]);
        git(&["commit", "-q", "-m", "First commit"]);
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        git(&["add", "a.txt"]);
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(dir.join("new.txt"), "x\n").unwrap();
        (Repo::discover(&dir).unwrap(), dir)
    }

    #[test]
    fn reads_history_status_and_both_diffs() {
        let (repo, _) = repo();
        let history = repo.history(10).unwrap();
        let commit = history
            .iter()
            .find_map(|line| line.commit.as_ref())
            .unwrap();
        assert_eq!(commit.subject, "First commit");
        assert!(history[0].graph.starts_with('*'));
        assert!(repo.show(&commit.sha).unwrap().contains("+one"));
        let status = repo.status().unwrap();
        assert_eq!(status.branch, "main");
        assert_eq!(status.staged, [("a.txt".to_string(), 'M')]);
        assert!(status.unstaged.contains(&("a.txt".to_string(), 'M')));
        assert!(status.unstaged.contains(&("new.txt".to_string(), '?')));
        assert!(repo.staged_diff().unwrap().contains("+two"));
        assert!(repo.working_diff().unwrap().contains("+three"));
    }

    #[test]
    fn reading_never_changes_the_repository() {
        let (repo, dir) = repo();
        let index = std::fs::read(dir.join(".git/index")).unwrap();
        let modified = std::fs::metadata(dir.join(".git/index"))
            .unwrap()
            .modified()
            .unwrap();
        for _ in 0..2 {
            repo.status().unwrap();
            repo.history(10).unwrap();
            repo.working_diff().unwrap();
            repo.staged_diff().unwrap();
        }
        assert_eq!(std::fs::read(dir.join(".git/index")).unwrap(), index);
        let after = std::fs::metadata(dir.join(".git/index"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(after, modified, "the index was not rewritten");
        assert!(!dir.join(".git/index.lock").exists());
    }

    #[test]
    fn stages_unstages_and_commits() {
        let (repo, dir) = repo();
        repo.stage("new.txt").unwrap();
        repo.stage("a.txt").unwrap();
        let status = repo.status().unwrap();
        assert!(status.staged.contains(&("new.txt".to_string(), 'A')));
        assert!(status.unstaged.is_empty(), "{:?}", status.unstaged);
        repo.unstage("new.txt").unwrap();
        assert!(
            repo.status()
                .unwrap()
                .unstaged
                .contains(&("new.txt".to_string(), '?'))
        );
        assert!(dir.join("new.txt").exists(), "unstaging keeps the file");
        let sha = repo.commit("Second commit\n\nWith a body").unwrap();
        assert_eq!(sha.len(), 40);
        let history = repo.history(5).unwrap();
        assert_eq!(history[0].commit.as_ref().unwrap().subject, "Second commit");
        assert!(repo.status().unwrap().staged.is_empty());
        assert!(repo.author().unwrap().contains('<'));
    }

    #[test]
    fn stages_and_unstages_one_line_or_one_hunk_of_a_file() {
        use crate::patch::FileDiff;
        let (repo, dir) = repo();
        repo.stage("a.txt").unwrap();
        repo.commit("Base").unwrap();
        let body: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        std::fs::write(dir.join("b.txt"), &body).unwrap();
        repo.stage("b.txt").unwrap();
        repo.commit("Add b").unwrap();
        let edited = body
            .replace("line 2\n", "line two\nline 2b\n")
            .replace("line 18\n", "line eighteen\n");
        std::fs::write(dir.join("b.txt"), &edited).unwrap();
        let working = diff_of(&repo.working_diff().unwrap(), "b.txt");
        let diff = FileDiff::parse(&working).unwrap();
        let line = |text: &str| working.lines().position(|l| l == text).unwrap();
        // One line: only "+line two" (and the removal it replaces stays unstaged).
        let patch = diff.line_patch(line("+line two"), false).unwrap();
        repo.apply_cached(&patch, false).unwrap();
        let staged = repo.staged_diff().unwrap();
        assert!(
            staged.contains("+line two") && !staged.contains("+line 2b"),
            "{staged}"
        );
        assert!(!staged.contains("-line 2\n"), "the removal was not chosen");
        assert_eq!(
            std::fs::read_to_string(dir.join("b.txt")).unwrap(),
            edited,
            "the working tree is untouched"
        );
        // A whole hunk: the change near line 18.
        let patch = diff.hunk_patch(line("+line eighteen")).unwrap();
        repo.apply_cached(&patch, false).unwrap();
        assert!(repo.staged_diff().unwrap().contains("+line eighteen"));
        // The same patch no longer fits the index, so git refuses it.
        assert!(repo.apply_cached(&patch, false).is_err());
        // Unstage just "+line two" from the staged diff.
        let staged = diff_of(&repo.staged_diff().unwrap(), "b.txt");
        let at = staged.lines().position(|l| l == "+line two").unwrap();
        let patch = FileDiff::parse(&staged)
            .unwrap()
            .line_patch(at, true)
            .unwrap();
        repo.apply_cached(&patch, true).unwrap();
        let staged = repo.staged_diff().unwrap();
        assert!(
            !staged.contains("line two") && staged.contains("+line eighteen"),
            "{staged}"
        );
    }

    #[test]
    fn branches_stash_and_rebase_run_through_git() {
        let (repo, dir) = repo();
        let op = |op: LocalOp, text: &str| repo.local_op(&op, text);
        let refs = repo.refs().unwrap();
        assert_eq!(refs.branches.len(), 1);
        assert!(refs.branches[0].current && refs.branches[0].name == "main");
        assert_eq!(refs.operation, None);
        // Stash everything, untracked files too, and get it back.
        op(LocalOp::StashPush, "wip").unwrap();
        assert!(
            !dir.join("new.txt").exists(),
            "the untracked file was stashed away"
        );
        let refs = repo.refs().unwrap();
        assert_eq!(refs.stashes[0].name, "stash@{0}");
        assert!(refs.stashes[0].message.contains("wip"));
        assert!(repo.stash_show("stash@{0}").unwrap().contains("+three"));
        assert!(repo.status().unwrap().unstaged.is_empty());
        // A stash made since the list was read moves stash@{0}: that is refused.
        let listed = repo.refs().unwrap().stashes[0].clone();
        std::fs::write(dir.join("a.txt"), "other\n").unwrap();
        op(LocalOp::StashPush, "newer").unwrap();
        assert!(op(LocalOp::StashDrop(listed.clone()), "").is_err());
        let newer = repo.refs().unwrap().stashes[0].clone();
        op(LocalOp::StashDrop(newer), "").unwrap();
        assert_eq!(repo.refs().unwrap().stashes[0].sha, listed.sha);
        op(LocalOp::StashPop(listed), "").unwrap();
        assert!(dir.join("new.txt").exists());
        assert!(repo.refs().unwrap().stashes.is_empty());
        repo.stage("a.txt").unwrap();
        repo.stage("new.txt").unwrap();
        repo.commit("Second").unwrap();
        // Branch names are checked before git sees them.
        assert!(op(LocalOp::CreateBranch, "-x").is_err());
        assert!(op(LocalOp::CreateBranch, "a..b").is_err());
        op(LocalOp::CreateBranch, "feature").unwrap();
        std::fs::write(dir.join("f.txt"), "feature\n").unwrap();
        repo.stage("f.txt").unwrap();
        repo.commit("Feature work").unwrap();
        op(LocalOp::Switch("main".into()), "").unwrap();
        std::fs::write(dir.join("m.txt"), "main\n").unwrap();
        repo.stage("m.txt").unwrap();
        repo.commit("Main work").unwrap();
        // An unmerged branch is not deleted by -d.
        assert!(op(LocalOp::DeleteBranch("feature".into()), "").is_err());
        op(LocalOp::Switch("feature".into()), "").unwrap();
        let plan = repo.rebase_plan("main").unwrap();
        assert_eq!(plan.incoming, 1);
        assert_eq!(plan.replay.len(), 1);
        assert!(plan.replay[0].ends_with("Feature work"));
        op(
            LocalOp::Rebase {
                onto: "main".into(),
                plan,
            },
            "",
        )
        .unwrap();
        assert!(dir.join("m.txt").exists() && dir.join("f.txt").exists());
        // A conflicting rebase stops, shows as in progress, and aborts cleanly.
        std::fs::write(dir.join("a.txt"), "feature side\n").unwrap();
        repo.stage("a.txt").unwrap();
        repo.commit("Feature edit").unwrap();
        op(LocalOp::Switch("main".into()), "").unwrap();
        std::fs::write(dir.join("a.txt"), "main side\n").unwrap();
        repo.stage("a.txt").unwrap();
        repo.commit("Main edit").unwrap();
        op(LocalOp::Switch("feature".into()), "").unwrap();
        let plan = repo.rebase_plan("main").unwrap();
        assert!(
            op(
                LocalOp::Rebase {
                    onto: "main".into(),
                    plan
                },
                ""
            )
            .is_err()
        );
        assert_eq!(repo.refs().unwrap().operation.as_deref(), Some("rebase"));
        op(LocalOp::RebaseAbort, "").unwrap();
        assert_eq!(repo.refs().unwrap().operation, None);
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "feature side\n"
        );
        assert!(repo.rebase_plan("--output=x").is_err());
    }

    #[test]
    fn a_stopped_rebase_shows_its_conflict_and_continues_once_resolved() {
        let (repo, dir) = repo();
        let op = |op: LocalOp, text: &str| repo.local_op(&op, text);
        repo.stage("a.txt").unwrap();
        repo.stage("new.txt").unwrap();
        repo.commit("Base").unwrap();
        op(LocalOp::CreateBranch, "topic").unwrap();
        std::fs::write(dir.join("a.txt"), "topic side\n").unwrap();
        repo.stage("a.txt").unwrap();
        repo.commit("Topic edit").unwrap();
        op(LocalOp::Switch("main".into()), "").unwrap();
        std::fs::write(dir.join("a.txt"), "main side\n").unwrap();
        repo.stage("a.txt").unwrap();
        repo.commit("Main edit").unwrap();
        op(LocalOp::Switch("topic".into()), "").unwrap();
        let plan = repo.rebase_plan("main").unwrap();
        assert!(
            op(
                LocalOp::Rebase {
                    onto: "main".into(),
                    plan
                },
                ""
            )
            .is_err()
        );
        let status = repo.status().unwrap();
        assert!(
            status.unstaged.contains(&("a.txt".to_string(), 'U')),
            "{status:?}"
        );
        assert!(diff_of(&repo.working_diff().unwrap(), "a.txt").contains("<<<<<<<"));
        // Resolve, stage the file whole, continue (no editor may open).
        std::fs::write(dir.join("a.txt"), "both sides\n").unwrap();
        repo.stage("a.txt").unwrap();
        op(LocalOp::RebaseContinue, "").unwrap();
        assert_eq!(repo.refs().unwrap().operation, None);
        let history = repo.history(3).unwrap();
        assert_eq!(history[0].commit.as_ref().unwrap().subject, "Topic edit");
    }

    #[test]
    fn one_line_of_a_crlf_file_stages_with_its_ending() {
        use crate::patch::FileDiff;
        let (repo, dir) = repo();
        repo.stage("a.txt").unwrap();
        repo.commit("Base").unwrap();
        std::fs::write(dir.join("w.txt"), "one\r\ntwo\r\nthree\r\n").unwrap();
        repo.stage("w.txt").unwrap();
        repo.commit("CRLF file").unwrap();
        std::fs::write(dir.join("w.txt"), "one\r\nTWO\r\nthree\r\nfour\r\n").unwrap();
        let working = diff_of(&repo.working_diff().unwrap(), "w.txt");
        let at = working
            .split_inclusive('\n')
            .position(|l| l == "+four\r\n")
            .unwrap();
        let patch = FileDiff::parse(&working)
            .unwrap()
            .line_patch(at, false)
            .unwrap();
        repo.apply_cached(&patch, false).unwrap();
        let staged = repo.git(&["show", ":w.txt"]).unwrap();
        assert_eq!(staged, "one\r\ntwo\r\nthree\r\nfour\r\n");
    }

    #[test]
    fn a_file_diff_is_cut_from_the_whole_diff() {
        let diff = "diff --git a/x b/x\n+one\ndiff --git a/y b/y\n+two\n";
        assert_eq!(diff_of(diff, "y"), "diff --git a/y b/y\n+two\n");
        assert_eq!(diff_of(diff, "z"), "");
    }

    #[test]
    fn refuses_anything_but_a_commit_id() {
        let (repo, _) = repo();
        assert!(repo.show("--output=/tmp/x").is_err());
    }

    #[test]
    fn finds_the_github_repository_of_a_remote_url() {
        for url in [
            "git@github.com:tlpinney/ghtui.git",
            "https://github.com/tlpinney/ghtui",
            "https://github.com/tlpinney/ghtui.git/",
            "ssh://git@github.com/tlpinney/ghtui.git",
        ] {
            assert_eq!(
                repo_from_url(url, "github.com").as_deref(),
                Some("tlpinney/ghtui")
            );
        }
        assert_eq!(repo_from_url("git@gitlab.com:a/b.git", "github.com"), None);
        assert_eq!(repo_from_url("https://github.com/a", "github.com"), None);
        for url in [
            "git@github.com:../x",
            "git@github.com:o/..",
            "https://github.com/o/a?x=1",
            "https://github.com/o/a#b",
            "https://github.com/o/a/b",
        ] {
            assert_eq!(repo_from_url(url, "github.com"), None, "{url}");
        }
    }

    #[test]
    fn parses_branch_counts_and_renames() {
        let status = parse_status(
            "# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n\
             2 R. N... 100644 100644 100644 abc def R100 new.rs\told.rs\n",
        );
        assert_eq!((status.ahead, status.behind), (2, 1));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!(status.staged, [("new.rs".to_string(), 'R')]);
    }
}
