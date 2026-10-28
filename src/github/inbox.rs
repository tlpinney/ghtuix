//! The Inbox reads: open PRs in three queues (one request) and recently merged PRs.

use super::Client;
use crate::model::{Checks, Decision, Person, PullRequest, Queue, Stance, add_reviewer};
use anyhow::Result;
use graphql_client::GraphQLQuery;

// GitHub's custom scalars, as the generated code names them.
type DateTime = String;
#[allow(clippy::upper_case_acronyms)]
type URI = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "schema/github.graphql",
    query_path = "src/github/queries/inbox.graphql",
    response_derives = "Debug"
)]
struct Inbox;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "schema/github.graphql",
    query_path = "src/github/queries/inbox.graphql",
    response_derives = "Debug"
)]
struct Merged;

/// Days the Merged tab looks back.
pub const MERGED_DAYS: i64 = 7;

/// Open PRs that you created, are assigned or are asked to review, newest update first.
pub fn open(client: &Client) -> Result<Vec<PullRequest>> {
    let search = |qualifier: &str| format!("is:pr is:open archived:false {qualifier}:@me");
    let data = client.graphql::<Inbox>(inbox::Variables {
        authored: search("author"),
        assigned: search("assignee"),
        review: search("review-requested"),
    })?;
    let queues = [
        (
            Queue::Authored,
            nodes(data.authored.nodes).filter_map(authored).collect(),
        ),
        (
            Queue::Assigned,
            nodes(data.assigned.nodes).filter_map(assigned).collect(),
        ),
        (
            Queue::Review,
            nodes(data.review.nodes).filter_map(review).collect(),
        ),
    ];
    let mut prs: Vec<PullRequest> = Vec::new();
    for (queue, found) in queues {
        let found: Vec<PullRequest> = found;
        for pr in found {
            match prs.iter_mut().find(|known| known.url == pr.url) {
                Some(known) => known.queues.push(queue),
                None => prs.push(PullRequest {
                    queues: vec![queue],
                    ..pr
                }),
            }
        }
    }
    prs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(prs)
}

/// PRs merged in the last [`MERGED_DAYS`] days that involve you, newest merge first.
pub fn merged(client: &Client) -> Result<Vec<PullRequest>> {
    let since = crate::time::date_days_ago(MERGED_DAYS);
    let data = client.graphql::<Merged>(merged::Variables {
        query: format!("is:pr is:merged archived:false involves:@me merged:>={since}"),
    })?;
    let mut prs: Vec<PullRequest> = nodes(data.search.nodes).filter_map(merged_pr).collect();
    prs.sort_by(|a, b| b.merged_at.cmp(&a.merged_at));
    Ok(prs)
}

/// A search connection's nodes, skipping GitHub's nulls.
fn nodes<T>(nodes: Option<Vec<Option<T>>>) -> impl Iterator<Item = T> {
    nodes.into_iter().flatten().flatten()
}

/// The same conversion for each generated copy of the `Pr` fragment.
macro_rules! convert {
    ($name:ident, $module:ident, $node:ident) => {
        fn $name(node: $module::$node) -> Option<PullRequest> {
            use $module::*;
            let $module::$node::PullRequest(pr) = node else {
                return None;
            };
            let person = |login: String, bot: bool| Person { login, bot };
            let author = pr.author.map_or_else(
                || person("ghost".into(), false),
                |a| person(a.login, matches!(a.on, PrAuthorOn::Bot)),
            );
            let decision = match pr.review_decision {
                Some(PullRequestReviewDecision::APPROVED) => Decision::Approved,
                Some(PullRequestReviewDecision::CHANGES_REQUESTED) => Decision::ChangesRequested,
                Some(PullRequestReviewDecision::REVIEW_REQUIRED) => Decision::Required,
                _ => Decision::None,
            };
            let rollup = pr
                .commits
                .nodes
                .into_iter()
                .flatten()
                .flatten()
                .next()
                .and_then(|node| node.commit.status_check_rollup);
            let checks = match rollup.map(|rollup| rollup.state) {
                None => Checks::None,
                Some(StatusState::SUCCESS) => Checks::Passing,
                Some(StatusState::FAILURE | StatusState::ERROR) => Checks::Failing,
                Some(_) => Checks::Pending,
            };
            let mut reviewers = Vec::new();
            for review in pr
                .latest_reviews
                .and_then(|c| c.nodes)
                .into_iter()
                .flatten()
                .flatten()
            {
                let stance = match review.state {
                    PullRequestReviewState::APPROVED => Stance::Approved,
                    PullRequestReviewState::CHANGES_REQUESTED => Stance::ChangesRequested,
                    PullRequestReviewState::COMMENTED => Stance::Reviewed,
                    PullRequestReviewState::DISMISSED => Stance::Dismissed,
                    _ => continue,
                };
                if let Some(a) = review.author {
                    let bot = matches!(a.on, PrLatestReviewsNodesAuthorOn::Bot);
                    add_reviewer(&mut reviewers, person(a.login, bot), stance);
                }
            }
            for request in pr
                .review_requests
                .and_then(|c| c.nodes)
                .into_iter()
                .flatten()
                .flatten()
            {
                use PrReviewRequestsNodesRequestedReviewer as R;
                let who = match request.requested_reviewer {
                    Some(R::User(user)) => person(user.login, false),
                    Some(R::Bot(bot)) => person(bot.login, true),
                    Some(R::Mannequin(m)) => person(m.login, false),
                    Some(R::Team(team)) => person(team.combined_slug, false),
                    _ => continue,
                };
                add_reviewer(&mut reviewers, who, Stance::Requested);
            }
            for comment in pr.comments.nodes.into_iter().flatten().flatten() {
                if let Some(a) = comment.author {
                    let bot = matches!(a.on, PrCommentsNodesAuthorOn::Bot);
                    add_reviewer(&mut reviewers, person(a.login, bot), Stance::Commented);
                }
            }
            reviewers.sort_by_key(|r| r.stance);
            Some(PullRequest {
                repo: pr.repository.name_with_owner,
                number: pr.number,
                title: pr.title,
                url: pr.url,
                body: pr.body_text,
                draft: pr.is_draft,
                author,
                created_at: pr.created_at,
                updated_at: pr.updated_at,
                merged_at: pr.merged_at,
                queues: Vec::new(),
                decision,
                checks,
                reviewers,
            })
        }
    };
}

convert!(authored, inbox, InboxAuthoredNodes);
convert!(assigned, inbox, InboxAssignedNodes);
convert!(review, inbox, InboxReviewNodes);
convert!(merged_pr, merged, MergedSearchNodes);
