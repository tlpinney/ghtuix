//! What the label and reviewer pickers offer, read when one opens.

use super::Client;
use crate::model::Label;
use anyhow::{Result, anyhow};
use graphql_client::GraphQLQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "schema/github.graphql",
    query_path = "src/github/queries/choices.graphql",
    response_derives = "Debug"
)]
struct RepoLabels;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "schema/github.graphql",
    query_path = "src/github/queries/choices.graphql",
    response_derives = "Debug"
)]
struct Assignable;

/// Pages of 100 read at most.
const MAX_PAGES: usize = 10;

fn split(repo: &str) -> Result<(&str, &str)> {
    repo.split_once('/')
        .ok_or_else(|| anyhow!("Not an owner/name repository: {repo}"))
}

/// Every label of `repo`.
pub fn labels(client: &Client, repo: &str) -> Result<Vec<Label>> {
    let (owner, name) = split(repo)?;
    let mut out = Vec::new();
    let mut after = None;
    for _ in 0..MAX_PAGES {
        let labels = client
            .graphql::<RepoLabels>(repo_labels::Variables {
                owner: owner.into(),
                name: name.into(),
                after: after.take(),
            })?
            .repository
            .and_then(|r| r.labels)
            .ok_or_else(|| anyhow!("{repo} is not visible to this account"))?;
        out.extend(
            labels
                .nodes
                .into_iter()
                .flatten()
                .flatten()
                .map(|label| Label {
                    id: label.id,
                    name: label.name,
                }),
        );
        if !labels.page_info.has_next_page {
            break;
        }
        after = labels.page_info.end_cursor;
    }
    Ok(out)
}

/// The logins that may be asked to review (or be assigned) in `repo`.
pub fn assignable(client: &Client, repo: &str) -> Result<Vec<String>> {
    let (owner, name) = split(repo)?;
    let mut out = Vec::new();
    let mut after = None;
    for _ in 0..MAX_PAGES {
        let users = client
            .graphql::<Assignable>(assignable::Variables {
                owner: owner.into(),
                name: name.into(),
                after: after.take(),
            })?
            .repository
            .ok_or_else(|| anyhow!("{repo} is not visible to this account"))?
            .assignable_users;
        out.extend(
            users
                .nodes
                .into_iter()
                .flatten()
                .flatten()
                .map(|user| user.login),
        );
        if !users.page_info.has_next_page {
            break;
        }
        after = users.page_info.end_cursor;
    }
    Ok(out)
}
