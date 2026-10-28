//! Read-only access to the local checkout through the installed `git`.
//!
//! Every command only reads. `--no-optional-locks` (and `GIT_OPTIONAL_LOCKS=0`) stop
//! `git status` from refreshing the index as a side effect; there are no prompts, pagers
//! or external diff tools, and paths are literal.

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

    /// Commits what is staged with `message`; returns the new commit's id.
    pub fn commit(&self, message: &str) -> Result<String> {
        write(&self.root, &["commit", "-q", "-F", "-"], Some(message))?;
        Ok(self.git(&["rev-parse", "HEAD"])?.trim().to_owned())
    }

    /// Changes not yet staged.
    pub fn working_diff(&self) -> Result<String> {
        self.git(&["diff", "--no-ext-diff"])
    }

    /// Changes staged for the next commit.
    pub fn staged_diff(&self) -> Result<String> {
        self.git(&["diff", "--no-ext-diff", "--cached"])
    }
}

/// Runs a read-only git command in `dir` and returns its output.
fn run(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(dir)
        .args([
            "--no-optional-locks",
            "--no-pager",
            "--literal-pathspecs",
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
    (!owner.is_empty() && !name.is_empty() && !name.contains('/'))
        .then(|| format!("{owner}/{name}"))
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
        .env_remove("GIT_EDITOR")
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
