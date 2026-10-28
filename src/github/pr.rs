//! A pull request page (one GraphQL query) and its diff (REST, by commit SHAs).

use super::Client;
use crate::model::{Check, Checks, Decision, Entry, FileChange, Person, Stance, Thread};
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

use pr_page::*;

/// Reads `repo#number` (`repo` is `owner/name`).
pub fn page(client: &Client, repo: &str, number: i64) -> Result<crate::model::PrPage> {
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| anyhow!("Not an owner/name repository: {repo}"))?;
    let data = client.graphql::<PrPage>(Variables {
        owner: owner.into(),
        name: name.into(),
        number,
    })?;
    let pr = data
        .repository
        .and_then(|repository| repository.pull_request)
        .ok_or_else(|| anyhow!("{repo}#{number} is not visible to this account"))?;
    Ok(convert(repo, pr))
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

fn convert(repo: &str, pr: PrPageRepositoryPullRequest) -> crate::model::PrPage {
    let author = pr.author.map_or_else(
        || person("ghost".into(), false),
        |a| {
            person(
                a.login,
                matches!(a.on, PrPageRepositoryPullRequestAuthorOn::Bot),
            )
        },
    );
    let mut conversation: Vec<Entry> = Vec::new();
    for comment in nodes(pr.comments.nodes) {
        let who = comment.author.map_or_else(
            || person("ghost".into(), false),
            |a| {
                let bot = matches!(a.on, PrPageRepositoryPullRequestCommentsNodesAuthorOn::Bot);
                person(a.login, bot)
            },
        );
        conversation.push(Entry {
            author: who,
            body: comment.body_text,
            at: comment.created_at,
            review: None,
        });
    }
    for review in nodes(pr.reviews.and_then(|reviews| reviews.nodes)) {
        let stance = match review.state {
            PullRequestReviewState::APPROVED => Stance::Approved,
            PullRequestReviewState::CHANGES_REQUESTED => Stance::ChangesRequested,
            PullRequestReviewState::DISMISSED => Stance::Dismissed,
            PullRequestReviewState::COMMENTED => Stance::Reviewed,
            _ => continue,
        };
        let who = review.author.map_or_else(
            || person("ghost".into(), false),
            |a| {
                let bot = matches!(a.on, PrPageRepositoryPullRequestReviewsNodesAuthorOn::Bot);
                person(a.login, bot)
            },
        );
        conversation.push(Entry {
            author: who,
            body: review.body_text,
            at: review.submitted_at.unwrap_or_default(),
            review: Some(stance),
        });
    }
    conversation.sort_by(|a, b| a.at.cmp(&b.at));
    let threads = nodes(pr.review_threads.nodes)
        .map(|thread| Thread {
            path: thread.path,
            line: thread.line,
            resolved: thread.is_resolved,
            comments: nodes(thread.comments.nodes)
                .map(|comment| Entry {
                    author: comment.author.map_or_else(
                        || person("ghost".into(), false),
                        |a| {
                            use PrPageRepositoryPullRequestReviewThreadsNodesCommentsNodesAuthorOn as On;
                            let bot = matches!(a.on, On::Bot);
                            person(a.login, bot)
                        },
                    ),
                    body: comment.body_text,
                    at: comment.created_at,
                    review: None,
                })
                .collect(),
        })
        .collect();
    let files = nodes(pr.files.and_then(|files| files.nodes))
        .map(|file| FileChange {
            path: file.path,
            additions: file.additions,
            deletions: file.deletions,
            change: format!("{:?}", file.change_type).to_lowercase(),
        })
        .collect();
    let rollup = nodes(pr.commits.nodes)
        .next()
        .and_then(|node| node.commit.status_check_rollup);
    let checks = rollup
        .map(|rollup| nodes(rollup.contexts.nodes).filter_map(check).collect())
        .unwrap_or_default();
    let decision = match pr.review_decision {
        Some(PullRequestReviewDecision::APPROVED) => Decision::Approved,
        Some(PullRequestReviewDecision::CHANGES_REQUESTED) => Decision::ChangesRequested,
        Some(PullRequestReviewDecision::REVIEW_REQUIRED) => Decision::Required,
        _ => Decision::None,
    };
    crate::model::PrPage {
        id: pr.id,
        repo: repo.into(),
        number: pr.number,
        title: pr.title,
        url: pr.url,
        body: pr.body_text,
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
        decision,
        conversation,
        threads,
        files,
        checks,
    }
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
            Some(Check {
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
                name: status.context,
                state,
                detail: format!("{:?}", status.state).to_lowercase(),
                url: status.target_url,
            })
        }
    }
}
