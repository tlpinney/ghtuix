//! Writes to a pull request: comments, reviews, line comments, replies and resolving.

use super::Client;
use crate::model::{MergeMethod, Side, WriteKind};
use anyhow::{Result, anyhow};
use graphql_client::GraphQLQuery;

#[allow(clippy::upper_case_acronyms)]
type URI = String;
#[allow(clippy::upper_case_acronyms)]
type GitObjectID = String;

macro_rules! mutation {
    ($name:ident) => {
        #[derive(GraphQLQuery)]
        #[graphql(
            schema_path = "schema/github.graphql",
            query_path = "src/github/queries/write.graphql",
            response_derives = "Debug"
        )]
        struct $name;
    };
}

mutation!(AddComment);
mutation!(AddReview);
mutation!(AddLineComment);
mutation!(MergePr);
mutation!(ReplyToThread);
mutation!(ResolveThread);
mutation!(UnresolveThread);
mutation!(ClosePr);
mutation!(ReopenPr);
mutation!(ToDraft);
mutation!(ReadyForReview);
mutation!(RenamePr);
mutation!(AddLabels);
mutation!(RemoveLabels);

/// Publishes `kind` with `body` on the PR with node id `pr_id`. Reviews and line comments
/// are pinned to `head_oid`, the head commit the user looked at. Returns the new item's URL,
/// or a short description when GitHub returns none (resolving).
pub fn publish(
    client: &Client,
    kind: &WriteKind,
    pr_id: &str,
    head_oid: &str,
    body: &str,
) -> Result<String> {
    let missing = || anyhow!("GitHub did not confirm the write");
    match kind {
        WriteKind::Comment => client
            .mutate::<AddComment>(add_comment::Variables {
                subject: pr_id.into(),
                body: body.into(),
            })?
            .add_comment
            .and_then(|added| added.comment_edge?.node)
            .map(|node| node.url)
            .ok_or_else(missing),
        WriteKind::Approve | WriteKind::RequestChanges => {
            let event = if *kind == WriteKind::Approve {
                add_review::PullRequestReviewEvent::APPROVE
            } else {
                add_review::PullRequestReviewEvent::REQUEST_CHANGES
            };
            client
                .mutate::<AddReview>(add_review::Variables {
                    pr: pr_id.into(),
                    commit: head_oid.into(),
                    event,
                    body: (!body.trim().is_empty()).then(|| body.into()),
                })?
                .add_pull_request_review
                .and_then(|added| added.pull_request_review)
                .map(|review| review.url)
                .ok_or_else(missing)
        }
        WriteKind::LineComment { path, line, side } => client
            .mutate::<AddLineComment>(add_line_comment::Variables {
                pr: pr_id.into(),
                commit: head_oid.into(),
                path: path.clone(),
                line: *line,
                side: match side {
                    Side::Left => add_line_comment::DiffSide::LEFT,
                    Side::Right => add_line_comment::DiffSide::RIGHT,
                },
                body: body.into(),
            })?
            .add_pull_request_review
            .and_then(|added| added.pull_request_review)
            .map(|review| review.url)
            .ok_or_else(missing),
        WriteKind::Reply { thread_id, .. } => client
            .mutate::<ReplyToThread>(reply_to_thread::Variables {
                thread: thread_id.clone(),
                body: body.into(),
            })?
            .add_pull_request_review_thread_reply
            .and_then(|added| added.comment)
            .map(|comment| comment.url)
            .ok_or_else(missing),
        WriteKind::Merge { method } => client
            .mutate::<MergePr>(merge_pr::Variables {
                pr: pr_id.into(),
                method: match method {
                    MergeMethod::Merge => merge_pr::PullRequestMergeMethod::MERGE,
                    MergeMethod::Squash => merge_pr::PullRequestMergeMethod::SQUASH,
                    MergeMethod::Rebase => merge_pr::PullRequestMergeMethod::REBASE,
                },
                head: head_oid.into(),
            })?
            .merge_pull_request
            .and_then(|done| done.pull_request)
            .filter(|pr| pr.merged)
            .map(|pr| pr.url)
            .ok_or_else(missing),
        WriteKind::Commit { .. } | WriteKind::Local { .. } => {
            anyhow::bail!("A local write never goes to GitHub")
        }
        WriteKind::Rerun { repo, run_id, .. } => {
            super::actions::rerun_failed(client, repo, *run_id)
                .map(|()| "re-run requested".to_string())
        }
        WriteKind::CancelRun { repo, run_id, .. } => {
            super::actions::cancel(client, repo, *run_id).map(|()| "cancel requested".to_string())
        }
        WriteKind::Resolve {
            thread_id,
            resolve: true,
            ..
        } => client
            .mutate::<ResolveThread>(resolve_thread::Variables {
                thread: thread_id.clone(),
            })?
            .resolve_review_thread
            .and_then(|done| done.thread)
            .filter(|thread| thread.is_resolved)
            .map(|_| "thread resolved".to_string())
            .ok_or_else(missing),
        WriteKind::Resolve { thread_id, .. } => client
            .mutate::<UnresolveThread>(unresolve_thread::Variables {
                thread: thread_id.clone(),
            })?
            .unresolve_review_thread
            .and_then(|done| done.thread)
            .filter(|thread| !thread.is_resolved)
            .map(|_| "thread unresolved".to_string())
            .ok_or_else(missing),
        WriteKind::MarkRead { thread_id } => {
            super::notifications::mark_read(client, thread_id).map(|()| "marked read".to_string())
        }
        WriteKind::MarkDone { thread_id } => {
            super::notifications::mark_done(client, thread_id).map(|()| "marked done".to_string())
        }
        WriteKind::Close => client
            .mutate::<ClosePr>(close_pr::Variables { pr: pr_id.into() })?
            .close_pull_request
            .and_then(|done| done.pull_request)
            .map(|pr| pr.url)
            .ok_or_else(missing),
        WriteKind::Reopen => client
            .mutate::<ReopenPr>(reopen_pr::Variables { pr: pr_id.into() })?
            .reopen_pull_request
            .and_then(|done| done.pull_request)
            .map(|pr| pr.url)
            .ok_or_else(missing),
        WriteKind::ToDraft => client
            .mutate::<ToDraft>(to_draft::Variables { pr: pr_id.into() })?
            .convert_pull_request_to_draft
            .and_then(|done| done.pull_request)
            .map(|pr| pr.url)
            .ok_or_else(missing),
        WriteKind::Ready => client
            .mutate::<ReadyForReview>(ready_for_review::Variables { pr: pr_id.into() })?
            .mark_pull_request_ready_for_review
            .and_then(|done| done.pull_request)
            .map(|pr| pr.url)
            .ok_or_else(missing),
        WriteKind::Title { .. } => {
            let title = new_title(body);
            if title.is_empty() {
                anyhow::bail!("A PR needs a title");
            }
            client
                .mutate::<RenamePr>(rename_pr::Variables {
                    pr: pr_id.into(),
                    title,
                })?
                .update_pull_request
                .and_then(|done| done.pull_request)
                .map(|pr| pr.url)
                .ok_or_else(missing)
        }
        WriteKind::Labels { add, remove } => {
            let ids =
                |labels: &[crate::model::Label]| labels.iter().map(|l| l.id.clone()).collect();
            if !add.is_empty() {
                client.mutate::<AddLabels>(add_labels::Variables {
                    pr: pr_id.into(),
                    labels: ids(add),
                })?;
            }
            if !remove.is_empty() {
                client.mutate::<RemoveLabels>(remove_labels::Variables {
                    pr: pr_id.into(),
                    labels: ids(remove),
                })?;
            }
            Ok("labels changed".into())
        }
        WriteKind::Reviewers {
            repo,
            number,
            add,
            remove,
        } => {
            let path = format!("repos/{repo}/pulls/{number}/requested_reviewers");
            if !add.is_empty() {
                let body = serde_json::json!({ "reviewers": add });
                client.rest_json("POST", &path, &body)?;
            }
            if !remove.is_empty() {
                let body = serde_json::json!({ "reviewers": remove });
                client.rest_json("DELETE", &path, &body)?;
            }
            Ok("review requests changed".into())
        }
    }
}

/// The title a rename sends: the compose box's first line, trimmed.
pub fn new_title(body: &str) -> String {
    body.lines().next().unwrap_or_default().trim().to_owned()
}
