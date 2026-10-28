//! A PR's commits and timeline, each read when its tab first shows.

use super::Client;
use crate::model::{Checks, Event, PrCommit};
use anyhow::{Result, anyhow};
use graphql_client::GraphQLQuery;

type DateTime = String;
#[allow(clippy::upper_case_acronyms)]
type GitObjectID = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "schema/github.graphql",
    query_path = "src/github/queries/history.graphql",
    response_derives = "Debug"
)]
struct PrCommits;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "schema/github.graphql",
    query_path = "src/github/queries/history.graphql",
    response_derives = "Debug"
)]
struct PrTimeline;

/// Pages of 100 read at most (GitHub keeps 250 commits per PR; timelines run long).
const MAX_PAGES: usize = 10;

fn nodes<T>(nodes: Option<Vec<Option<T>>>) -> impl Iterator<Item = T> {
    nodes.into_iter().flatten().flatten()
}

fn split(repo: &str) -> Result<(&str, &str)> {
    repo.split_once('/')
        .ok_or_else(|| anyhow!("Not an owner/name repository: {repo}"))
}

/// Every commit of the PR, oldest first.
pub fn commits(client: &Client, repo: &str, number: i64) -> Result<Vec<PrCommit>> {
    use pr_commits::StatusState;
    let (owner, name) = split(repo)?;
    let mut out = Vec::new();
    let mut after = None;
    for _ in 0..MAX_PAGES {
        let data = client.graphql::<PrCommits>(pr_commits::Variables {
            owner: owner.into(),
            name: name.into(),
            number,
            after: after.take(),
        })?;
        let commits = data
            .repository
            .and_then(|r| r.pull_request)
            .ok_or_else(|| anyhow!("{repo}#{number} is not visible to this account"))?
            .commits;
        for node in nodes(commits.nodes) {
            let commit = node.commit;
            let author = commit.author.map_or_else(String::new, |author| {
                author
                    .user
                    .map(|user| user.login)
                    .or(author.name)
                    .unwrap_or_default()
            });
            let checks = match commit.status_check_rollup.map(|rollup| rollup.state) {
                None => Checks::None,
                Some(StatusState::SUCCESS) => Checks::Passing,
                Some(StatusState::FAILURE | StatusState::ERROR) => Checks::Failing,
                Some(_) => Checks::Pending,
            };
            out.push(PrCommit {
                oid: commit.oid,
                headline: commit.message_headline,
                author,
                date: commit.committed_date,
                checks,
            });
        }
        if !commits.page_info.has_next_page {
            break;
        }
        after = commits.page_info.end_cursor;
    }
    Ok(out)
}

/// What happened on the PR, oldest first: one line of text per event.
pub fn timeline(client: &Client, repo: &str, number: i64) -> Result<Vec<Event>> {
    use pr_timeline::PrTimelineRepositoryPullRequestTimelineItemsNodes as Node;
    let (owner, name) = split(repo)?;
    let mut out = Vec::new();
    let mut after = None;
    let who = |login: Option<String>| login.unwrap_or_else(|| "ghost".into());
    for _ in 0..MAX_PAGES {
        let data = client.graphql::<PrTimeline>(pr_timeline::Variables {
            owner: owner.into(),
            name: name.into(),
            number,
            after: after.take(),
        })?;
        let items = data
            .repository
            .and_then(|r| r.pull_request)
            .ok_or_else(|| anyhow!("{repo}#{number} is not visible to this account"))?
            .timeline_items;
        for node in nodes(items.nodes) {
            let event = |at: String, actor: Option<String>, text: String| Event {
                at,
                actor: who(actor),
                text,
            };
            let short = |oid: &str| oid.chars().take(8).collect::<String>();
            let item = match node {
                Node::PullRequestCommit(c) => {
                    let author = c
                        .commit
                        .author
                        .and_then(|a| a.user.map(|u| u.login).or(a.name));
                    let text = format!(
                        "pushed {} {}",
                        short(&c.commit.oid),
                        c.commit.message_headline
                    );
                    event(c.commit.committed_date, author, text)
                }
                Node::HeadRefForcePushedEvent(e) => {
                    let to = e
                        .after_commit
                        .map_or(String::new(), |c| format!(" to {}", short(&c.oid)));
                    event(
                        e.created_at,
                        e.actor.map(|a| a.login),
                        format!("force-pushed{to}"),
                    )
                }
                Node::PullRequestReview(r) => {
                    let state = format!("{:?}", r.state).to_lowercase().replace('_', " ");
                    event(
                        r.created_at,
                        r.author.map(|a| a.login),
                        format!("reviewed: {state}"),
                    )
                }
                Node::IssueComment(c) => {
                    event(c.created_at, c.author.map(|a| a.login), "commented".into())
                }
                Node::ReviewRequestedEvent(e) => {
                    use pr_timeline::PrTimelineRepositoryPullRequestTimelineItemsNodesOnReviewRequestedEventRequestedReviewer as R;
                    let whom = match e.requested_reviewer {
                        Some(R::User(u)) => u.login,
                        Some(R::Bot(b)) => b.login,
                        Some(R::Team(t)) => t.combined_slug,
                        _ => "someone".into(),
                    };
                    let text = format!("requested a review from {whom}");
                    event(e.created_at, e.actor.map(|a| a.login), text)
                }
                Node::ReviewDismissedEvent(e) => event(
                    e.created_at,
                    e.actor.map(|a| a.login),
                    "dismissed a review".into(),
                ),
                Node::LabeledEvent(e) => {
                    let text = format!("added the label {}", e.label.name);
                    event(e.created_at, e.actor.map(|a| a.login), text)
                }
                Node::UnlabeledEvent(e) => {
                    let text = format!("removed the label {}", e.label.name);
                    event(e.created_at, e.actor.map(|a| a.login), text)
                }
                Node::RenamedTitleEvent(e) => {
                    let text = format!(
                        "renamed it from “{}” to “{}”",
                        e.previous_title, e.current_title
                    );
                    event(e.created_at, e.actor.map(|a| a.login), text)
                }
                Node::AssignedEvent(e) => {
                    use pr_timeline::PrTimelineRepositoryPullRequestTimelineItemsNodesOnAssignedEventAssignee as A;
                    let whom = match e.assignee {
                        Some(A::User(u)) => u.login,
                        Some(A::Bot(b)) => b.login,
                        _ => "someone".into(),
                    };
                    event(
                        e.created_at,
                        e.actor.map(|a| a.login),
                        format!("assigned {whom}"),
                    )
                }
                Node::ReadyForReviewEvent(e) => event(
                    e.created_at,
                    e.actor.map(|a| a.login),
                    "marked it ready for review".into(),
                ),
                Node::ConvertToDraftEvent(e) => event(
                    e.created_at,
                    e.actor.map(|a| a.login),
                    "converted it to a draft".into(),
                ),
                Node::ClosedEvent(e) => {
                    event(e.created_at, e.actor.map(|a| a.login), "closed it".into())
                }
                Node::ReopenedEvent(e) => {
                    event(e.created_at, e.actor.map(|a| a.login), "reopened it".into())
                }
                Node::MergedEvent(e) => {
                    let text = format!("merged it into {}", e.merge_ref_name);
                    event(e.created_at, e.actor.map(|a| a.login), text)
                }
                _ => continue,
            };
            out.push(item);
        }
        if !items.page_info.has_next_page {
            break;
        }
        after = items.page_info.end_cursor;
    }
    Ok(out)
}
