//! The Search view's read: GitHub's issue and pull request search, one page of 50.

use super::Client;
use anyhow::{Result, anyhow};
use graphql_client::GraphQLQuery;
use serde::{Deserialize, Serialize};

type DateTime = String;
#[allow(clippy::upper_case_acronyms)]
type URI = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "schema/github.graphql",
    query_path = "src/github/queries/search.graphql",
    response_derives = "Debug"
)]
struct SearchIssues;

/// One search result: a pull request or an issue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    pub repo: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub pr: bool,
    /// `open`, `closed`, `merged`, or `draft` for an open draft PR.
    pub state: String,
    pub author: String,
    pub updated_at: String,
}

impl Hit {
    pub fn reference(&self) -> String {
        format!("{}#{}", self.repo, self.number)
    }
}

/// The first 50 results of `query`, and how many GitHub found in all.
pub fn search(client: &Client, query: &str) -> Result<(Vec<Hit>, i64)> {
    use search_issues::SearchIssuesSearchNodes as Node;
    if query.trim().is_empty() {
        return Err(anyhow!("Type a search first"));
    }
    let search = client
        .graphql::<SearchIssues>(search_issues::Variables {
            query: query.into(),
        })?
        .search;
    let login = |author: Option<String>| author.unwrap_or_else(|| "ghost".into());
    let hits = search
        .nodes
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|node| match node {
            Node::PullRequest(pr) => Some(Hit {
                repo: pr.repository.name_with_owner,
                number: pr.number,
                title: pr.title,
                url: pr.url,
                pr: true,
                state: if pr.is_draft && format!("{:?}", pr.state) == "OPEN" {
                    "draft".into()
                } else {
                    format!("{:?}", pr.state).to_lowercase()
                },
                author: login(pr.author.map(|a| a.login)),
                updated_at: pr.updated_at,
            }),
            Node::Issue(issue) => Some(Hit {
                repo: issue.repository.name_with_owner,
                number: issue.number,
                title: issue.title,
                url: issue.url,
                pr: false,
                state: format!("{:?}", issue.issue_state).to_lowercase(),
                author: login(issue.author.map(|a| a.login)),
                updated_at: issue.updated_at,
            }),
            _ => None,
        })
        .collect();
    Ok((hits, search.issue_count))
}
