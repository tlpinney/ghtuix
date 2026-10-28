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
    fn refuses_anything_but_a_commit_id() {
        let (repo, _) = repo();
        assert!(repo.show("--output=/tmp/x").is_err());
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
