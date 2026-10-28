//! Writes: a conversation comment or a review on a pull request.

use super::Client;
use crate::model::WriteKind;
use anyhow::{Result, anyhow};
use graphql_client::GraphQLQuery;

#[allow(clippy::upper_case_acronyms)]
type URI = String;
#[allow(clippy::upper_case_acronyms)]
type GitObjectID = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "schema/github.graphql",
    query_path = "src/github/queries/write.graphql",
    response_derives = "Debug"
)]
struct AddComment;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "schema/github.graphql",
    query_path = "src/github/queries/write.graphql",
    response_derives = "Debug"
)]
struct AddReview;

/// Publishes `body` as `kind` on the PR with node id `pr_id`; a review is pinned to the
/// head commit the user looked at. Returns the new comment's or review's URL.
pub fn publish(
    client: &Client,
    kind: WriteKind,
    pr_id: &str,
    head_oid: &str,
    body: &str,
) -> Result<String> {
    let event = match kind {
        WriteKind::Comment => {
            let data = client.mutate::<AddComment>(add_comment::Variables {
                subject: pr_id.into(),
                body: body.into(),
            })?;
            return data
                .add_comment
                .and_then(|added| added.comment_edge)
                .and_then(|edge| edge.node)
                .map(|node| node.url)
                .ok_or_else(|| anyhow!("GitHub did not return the new comment"));
        }
        WriteKind::Approve => add_review::PullRequestReviewEvent::APPROVE,
        WriteKind::RequestChanges => add_review::PullRequestReviewEvent::REQUEST_CHANGES,
    };
    let data = client.mutate::<AddReview>(add_review::Variables {
        pr: pr_id.into(),
        commit: head_oid.into(),
        event,
        body: (!body.trim().is_empty()).then(|| body.into()),
    })?;
    data.add_pull_request_review
        .and_then(|added| added.pull_request_review)
        .map(|review| review.url)
        .ok_or_else(|| anyhow!("GitHub did not return the new review"))
}
