//! The app's own types, independent of the GitHub API shapes.

use serde::{Deserialize, Serialize};

/// Which Inbox queue a pull request came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Queue {
    Authored,
    Assigned,
    Review,
}

/// GitHub's review decision for a pull request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Approved,
    ChangesRequested,
    Required,
    None,
}

/// The head commit's combined check state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Checks {
    Passing,
    Failing,
    Pending,
    None,
}

/// Where someone stands on a pull request, most significant first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Stance {
    ChangesRequested,
    Approved,
    Reviewed,
    Dismissed,
    Requested,
    Commented,
}

impl Stance {
    pub fn label(self) -> &'static str {
        match self {
            Self::ChangesRequested => "changes requested",
            Self::Approved => "approved",
            Self::Reviewed => "reviewed",
            Self::Dismissed => "review dismissed",
            Self::Requested => "requested",
            Self::Commented => "commented",
        }
    }
}

/// Which side of a diff a line is on: the base (`Left`) or the head (`Right`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
}

/// How a pull request is merged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    pub const ALL: [(MergeMethod, char, &'static str); 3] = [
        (MergeMethod::Merge, 'm', "Merge commit"),
        (MergeMethod::Squash, 's', "Squash"),
        (MergeMethod::Rebase, 'r', "Rebase"),
    ];

    pub fn label(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(m, ..)| *m == self)
            .map_or("", |(_, _, label)| label)
    }
}

/// What a write does to a pull request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteKind {
    Comment,
    Approve,
    RequestChanges,
    /// A review comment on one line of the diff.
    LineComment {
        path: String,
        line: i64,
        side: Side,
    },
    /// A reply in an existing review thread.
    Reply {
        thread_id: String,
        place: String,
    },
    /// Resolve (or, with `false`, unresolve) a review thread.
    Resolve {
        thread_id: String,
        place: String,
        resolve: bool,
    },
    /// Merge the pull request (only if its head is still the one viewed).
    Merge {
        method: MergeMethod,
    },
}

impl WriteKind {
    pub fn label(&self) -> String {
        match self {
            Self::Comment => "Comment".into(),
            Self::Approve => "Approve".into(),
            Self::RequestChanges => "Request changes".into(),
            Self::LineComment { path, line, .. } => format!("Comment on {path}:{line}"),
            Self::Reply { place, .. } => format!("Reply on {place}"),
            Self::Resolve { place, resolve, .. } => {
                let verb = if *resolve { "Resolve" } else { "Unresolve" };
                format!("{verb} the thread on {place}")
            }
            Self::Merge { method } => format!("Merge ({})", method.label().to_lowercase()),
        }
    }

    /// Whether the write has text at all (resolving a thread has none).
    pub fn has_text(&self) -> bool {
        !matches!(self, Self::Resolve { .. } | Self::Merge { .. })
    }

    /// Whether the text may be empty (an approval needs no words).
    pub fn body_optional(&self) -> bool {
        matches!(
            self,
            Self::Approve | Self::Resolve { .. } | Self::Merge { .. }
        )
    }
}

/// A GitHub user, bot or team (a team's login is `org/slug`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub login: String,
    pub bot: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviewer {
    pub person: Person,
    pub stance: Stance,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub repo: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub body: String,
    pub draft: bool,
    pub author: Person,
    pub created_at: String,
    pub updated_at: String,
    pub merged_at: Option<String>,
    pub queues: Vec<Queue>,
    pub decision: Decision,
    pub checks: Checks,
    /// Reviews first, then requests, then commenters; one entry per login.
    pub reviewers: Vec<Reviewer>,
}

impl PullRequest {
    pub fn reference(&self) -> String {
        format!("{}#{}", self.repo, self.number)
    }

    /// The STATUS cell: review decision and checks, in words.
    pub fn status(&self) -> String {
        if let Some(merged) = &self.merged_at {
            return format!("Merged {} ago", crate::time::age(merged));
        }
        let review = match self.decision {
            Decision::Approved => "approved",
            Decision::ChangesRequested => "changes requested",
            Decision::Required => "review required",
            Decision::None => "no review decision",
        };
        let checks = match self.checks {
            Checks::Passing => "checks pass",
            Checks::Failing => "checks fail",
            Checks::Pending => "checks pending",
            Checks::None => "no checks",
        };
        format!("{review}, {checks}")
    }
}

/// One item of a PR's conversation: a comment or a submitted review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub author: Person,
    pub body: String,
    pub at: String,
    /// `None` for a comment; the review's stance for a review.
    pub review: Option<Stance>,
}

/// A review thread on a line of a changed file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    /// GitHub's node id, which replies and resolving address.
    pub id: String,
    pub path: String,
    pub line: Option<i64>,
    pub resolved: bool,
    pub comments: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub additions: i64,
    pub deletions: i64,
    /// `added`, `modified`, `renamed`, …
    pub change: String,
}

/// One check run or commit status on the head commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub state: Checks,
    /// GitHub's wording for the state, such as `success` or `in progress`.
    pub detail: String,
    pub url: Option<String>,
}

/// Everything the PR view shows, read in one query (the diff comes separately).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrPage {
    /// GitHub's node id, which writes address.
    pub id: String,
    pub repo: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub body: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
    pub draft: bool,
    pub author: Person,
    pub created_at: String,
    pub updated_at: String,
    pub head_ref: String,
    pub base_ref: String,
    pub head_oid: String,
    pub base_oid: String,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    /// `mergeable`, `conflicting` or `unknown`.
    pub mergeable: String,
    /// GitHub's merge state: `clean`, `blocked`, `behind`, `dirty`, `unstable`, …
    pub merge_state: String,
    /// The merge methods the repository allows.
    pub merge_methods: Vec<MergeMethod>,
    pub decision: Decision,
    /// Comments and reviews, oldest first.
    pub conversation: Vec<Entry>,
    pub threads: Vec<Thread>,
    pub files: Vec<FileChange>,
    pub checks: Vec<Check>,
}

/// Adds `person` with `stance`, keeping one entry per login (the most significant stance).
pub fn add_reviewer(reviewers: &mut Vec<Reviewer>, person: Person, stance: Stance) {
    if person.login.is_empty() {
        return;
    }
    match reviewers
        .iter_mut()
        .find(|r| r.person.login.eq_ignore_ascii_case(&person.login))
    {
        Some(existing) => existing.stance = existing.stance.min(stance),
        None => reviewers.push(Reviewer { person, stance }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(login: &str) -> Person {
        Person {
            login: login.into(),
            bot: false,
        }
    }

    #[test]
    fn reviewers_keep_one_entry_with_the_strongest_stance() {
        let mut reviewers = Vec::new();
        add_reviewer(&mut reviewers, person("alice"), Stance::Commented);
        add_reviewer(&mut reviewers, person("Alice"), Stance::Approved);
        add_reviewer(&mut reviewers, person("bob"), Stance::Requested);
        add_reviewer(&mut reviewers, person(""), Stance::Approved);
        let seen: Vec<_> = reviewers
            .iter()
            .map(|r| (r.person.login.as_str(), r.stance))
            .collect();
        assert_eq!(
            seen,
            [("alice", Stance::Approved), ("bob", Stance::Requested)]
        );
    }
}
