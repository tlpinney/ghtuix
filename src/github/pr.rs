//! A pull request page (one GraphQL query) and its diff (REST, by commit SHAs).

use super::Client;
use crate::model::{
    Check, Checks, Decision, Entry, FileChange, Label, MergeMethod, Person, Stance, Thread,
};
use anyhow::{Result, anyhow};
use graphql_client::GraphQLQuery;

type DateTime = String;
#[allow(clippy::upper_case_acronyms)]
type URI = String;
#[allow(clippy::upper_case_acronyms)]
type GitObjectID = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "schema/github.graphql",
    query_path = "src/github/queries/pr.graphql",
    response_derives = "Debug"
)]
struct PrPage;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "schema/github.graphql",
    query_path = "src/github/queries/pr.graphql",
    response_derives = "Debug"
)]
struct PrMore;

use pr_page::*;

/// Most follow-up pages read for one PR (each brings up to 100 more of every list).
const MAX_MORE: usize = 10;

/// Where each long list of a PR continues: `(more to read, cursor after the last read)`.
#[derive(Clone, Debug, Default)]
struct Cursors {
    comments: (bool, Option<String>),
    reviews: (bool, Option<String>),
    threads: (bool, Option<String>),
    files: (bool, Option<String>),
}

impl Cursors {
    fn more(&self) -> bool {
        self.comments.0 || self.reviews.0 || self.threads.0 || self.files.0
    }
}

/// The page info of a connection as `(has next page, end cursor)`.
macro_rules! cursor {
    ($connection:expr) => {
        (
            $connection.page_info.has_next_page,
            $connection.page_info.end_cursor.clone(),
        )
    };
}

/// Converting the shared fragments, for each query's generated copy of them.
macro_rules! fragment_conversions {
    ($m:ident) => {
        pub(super) fn comment(c: super::$m::CommentNode) -> Entry {
            use super::$m::CommentNodeAuthorOn as On;
            Entry {
                author: c
                    .author
                    .map_or_else(ghost, |a| person(a.login, matches!(a.on, On::Bot))),
                body: c.body,
                at: c.created_at,
                review: None,
            }
        }

        pub(super) fn review(r: super::$m::ReviewNode) -> Option<Entry> {
            use super::$m::{PullRequestReviewState as S, ReviewNodeAuthorOn as On};
            let stance = match r.state {
                S::APPROVED => Stance::Approved,
                S::CHANGES_REQUESTED => Stance::ChangesRequested,
                S::DISMISSED => Stance::Dismissed,
                S::COMMENTED => Stance::Reviewed,
                _ => return None,
            };
            Some(Entry {
                author: r
                    .author
                    .map_or_else(ghost, |a| person(a.login, matches!(a.on, On::Bot))),
                body: r.body,
                at: r.submitted_at.unwrap_or_default(),
                review: Some(stance),
            })
        }

        pub(super) fn thread(t: super::$m::ThreadNode) -> Thread {
            use super::$m::ThreadNodeCommentsNodesAuthorOn as On;
            Thread {
                id: t.id,
                path: t.path,
                line: t.line,
                resolved: t.is_resolved,
                comments: nodes(t.comments.nodes)
                    .map(|c| Entry {
                        author: c
                            .author
                            .map_or_else(ghost, |a| person(a.login, matches!(a.on, On::Bot))),
                        body: c.body,
                        at: c.created_at,
                        review: None,
                    })
                    .collect(),
            }
        }

        pub(super) fn file(f: super::$m::FileNode) -> FileChange {
            FileChange {
                path: f.path,
                additions: f.additions,
                deletions: f.deletions,
                change: format!("{:?}", f.change_type).to_lowercase(),
            }
        }
    };
}

mod page_nodes {
    use super::*;
    fragment_conversions!(pr_page);
}

mod more_nodes {
    use super::*;
    fragment_conversions!(pr_more);
}

/// Reads `repo#number` (`repo` is `owner/name`), following every long list to its end.
pub fn page(client: &Client, repo: &str, number: i64) -> Result<crate::model::PrPage> {
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| anyhow!("Not an owner/name repository: {repo}"))?;
    let data = client.graphql::<PrPage>(Variables {
        owner: owner.into(),
        name: name.into(),
        number,
    })?;
    let repository = data
        .repository
        .ok_or_else(|| anyhow!("{repo} is not visible to this account"))?;
    let methods = [
        (repository.merge_commit_allowed, MergeMethod::Merge),
        (repository.squash_merge_allowed, MergeMethod::Squash),
        (repository.rebase_merge_allowed, MergeMethod::Rebase),
    ]
    .into_iter()
    .filter_map(|(allowed, method)| allowed.then_some(method))
    .collect();
    let pr = repository
        .pull_request
        .ok_or_else(|| anyhow!("{repo}#{number} is not visible to this account"))?;
    let (mut page, mut cursors) = convert(repo, pr, methods);
    for _ in 0..MAX_MORE {
        if !cursors.more() {
            break;
        }
        cursors = more(client, owner, name, number, &cursors, &mut page)?;
    }
    page.conversation.sort_by(|a, b| a.at.cmp(&b.at));
    Ok(page)
}

/// Reads the next page of every list that has one, adding them to `page`.
fn more(
    client: &Client,
    owner: &str,
    name: &str,
    number: i64,
    cursors: &Cursors,
    page: &mut crate::model::PrPage,
) -> Result<Cursors> {
    let data = client.graphql::<PrMore>(pr_more::Variables {
        owner: owner.into(),
        name: name.into(),
        number,
        comments: cursors.comments.1.clone(),
        reviews: cursors.reviews.1.clone(),
        threads: cursors.threads.1.clone(),
        files: cursors.files.1.clone(),
    })?;
    let pr = data
        .repository
        .and_then(|repository| repository.pull_request)
        .ok_or_else(|| anyhow!("The PR went away while its pages were read"))?;
    let mut next = cursors.clone();
    // Only lists that still had pages take what came back.
    if cursors.comments.0 {
        next.comments = cursor!(pr.comments);
        page.conversation
            .extend(nodes(pr.comments.nodes).map(more_nodes::comment));
    }
    if cursors.reviews.0
        && let Some(reviews) = pr.reviews
    {
        next.reviews = cursor!(reviews);
        page.conversation
            .extend(nodes(reviews.nodes).filter_map(more_nodes::review));
    }
    if cursors.threads.0 {
        next.threads = cursor!(pr.review_threads);
        page.threads
            .extend(nodes(pr.review_threads.nodes).map(more_nodes::thread));
    }
    if cursors.files.0
        && let Some(files) = pr.files
    {
        next.files = cursor!(files);
        page.files.extend(nodes(files.nodes).map(more_nodes::file));
    }
    Ok(next)
}

/// The log of a GitHub Actions job: GitHub redirects to short-lived blob storage, and the
/// token is never sent along on that redirect.
pub fn job_log(client: &Client, repo: &str, job_id: i64) -> Result<String> {
    client.rest_text(
        &format!("repos/{repo}/actions/jobs/{job_id}/logs"),
        "application/vnd.github+json",
    )
}

/// The unified diff from `base` to `head` (commit SHAs, so it never changes).
pub fn diff(client: &Client, repo: &str, number: i64) -> Result<String> {
    client.rest_text(
        &format!("repos/{repo}/pulls/{number}"),
        "application/vnd.github.diff",
    )
}

fn nodes<T>(nodes: Option<Vec<Option<T>>>) -> impl Iterator<Item = T> {
    nodes.into_iter().flatten().flatten()
}

fn person(login: String, bot: bool) -> Person {
    Person { login, bot }
}

fn ghost() -> Person {
    person("ghost".into(), false)
}

fn convert(
    repo: &str,
    pr: PrPageRepositoryPullRequest,
    merge_methods: Vec<MergeMethod>,
) -> (crate::model::PrPage, Cursors) {
    let author = pr.author.map_or_else(ghost, |a| {
        person(
            a.login,
            matches!(a.on, PrPageRepositoryPullRequestAuthorOn::Bot),
        )
    });
    let mut cursors = Cursors {
        comments: cursor!(pr.comments),
        threads: cursor!(pr.review_threads),
        ..Cursors::default()
    };
    let mut conversation: Vec<Entry> = nodes(pr.comments.nodes).map(page_nodes::comment).collect();
    if let Some(reviews) = pr.reviews {
        cursors.reviews = cursor!(reviews);
        conversation.extend(nodes(reviews.nodes).filter_map(page_nodes::review));
    }
    let threads = nodes(pr.review_threads.nodes)
        .map(page_nodes::thread)
        .collect();
    let files = match pr.files {
        Some(files) => {
            cursors.files = cursor!(files);
            nodes(files.nodes).map(page_nodes::file).collect()
        }
        None => Vec::new(),
    };
    let rollup = nodes(pr.commits.nodes)
        .next()
        .and_then(|node| node.commit.status_check_rollup);
    let checks = rollup
        .map(|rollup| nodes(rollup.contexts.nodes).filter_map(check).collect())
        .unwrap_or_default();
    let labels = nodes(pr.labels.and_then(|labels| labels.nodes))
        .map(|label| Label {
            id: label.id,
            name: label.name,
        })
        .collect();
    let requested = nodes(pr.review_requests.and_then(|requests| requests.nodes))
        .filter_map(|request| {
            use PrPageRepositoryPullRequestReviewRequestsNodesRequestedReviewer as R;
            match request.requested_reviewer? {
                R::User(user) => Some(user.login),
                R::Bot(bot) => Some(bot.login),
                _ => None,
            }
        })
        .collect();
    let decision = match pr.review_decision {
        Some(PullRequestReviewDecision::APPROVED) => Decision::Approved,
        Some(PullRequestReviewDecision::CHANGES_REQUESTED) => Decision::ChangesRequested,
        Some(PullRequestReviewDecision::REVIEW_REQUIRED) => Decision::Required,
        _ => Decision::None,
    };
    let page = crate::model::PrPage {
        id: pr.id,
        repo: repo.into(),
        number: pr.number,
        title: pr.title,
        url: pr.url,
        body: pr.body,
        state: format!("{:?}", pr.state).to_lowercase(),
        draft: pr.is_draft,
        author,
        created_at: pr.created_at,
        updated_at: pr.updated_at,
        head_ref: pr.head_ref_name,
        base_ref: pr.base_ref_name,
        head_oid: pr.head_ref_oid,
        base_oid: pr.base_ref_oid,
        additions: pr.additions,
        deletions: pr.deletions,
        changed_files: pr.changed_files,
        mergeable: format!("{:?}", pr.mergeable).to_lowercase(),
        merge_state: format!("{:?}", pr.merge_state_status).to_lowercase(),
        merge_methods,
        labels,
        requested,
        decision,
        conversation,
        threads,
        files,
        checks,
    };
    (page, cursors)
}

fn check(
    node: PrPageRepositoryPullRequestCommitsNodesCommitStatusCheckRollupContextsNodes,
) -> Option<Check> {
    use PrPageRepositoryPullRequestCommitsNodesCommitStatusCheckRollupContextsNodes as Node;
    match node {
        Node::CheckRun(run) => {
            let (state, detail) = match (&run.conclusion, &run.status) {
                (Some(CheckConclusionState::SUCCESS), _) => (Checks::Passing, "success".into()),
                (Some(CheckConclusionState::NEUTRAL | CheckConclusionState::SKIPPED), _) => (
                    Checks::Passing,
                    format!("{:?}", run.conclusion.as_ref()?).to_lowercase(),
                ),
                (Some(conclusion), _) => {
                    (Checks::Failing, format!("{conclusion:?}").to_lowercase())
                }
                (None, status) => (Checks::Pending, format!("{status:?}").to_lowercase()),
            };
            // Only GitHub Actions check runs are jobs with a log to download.
            let actions = run
                .check_suite
                .app
                .is_some_and(|app| app.slug == "github-actions");
            Some(Check {
                job_id: run.database_id.filter(|_| actions),
                name: run.name,
                state,
                detail: detail.replace('_', " "),
                url: run.details_url,
            })
        }
        Node::StatusContext(status) => {
            let state = match status.state {
                StatusState::SUCCESS => Checks::Passing,
                StatusState::FAILURE | StatusState::ERROR => Checks::Failing,
                _ => Checks::Pending,
            };
            Some(Check {
                job_id: None,
                name: status.context,
                state,
                detail: format!("{:?}", status.state).to_lowercase(),
                url: status.target_url,
            })
        }
    }
}
