//! The Dashboard's reads: a repository's PRs over a window (one GraphQL request) and its
//! workflow runs over the same window (one REST read).

use super::Client;
use super::actions::Run;
use anyhow::{Context, Result, anyhow};
use graphql_client::GraphQLQuery;
use serde::{Deserialize, Serialize};

type DateTime = String;
#[allow(clippy::upper_case_acronyms)]
type URI = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "schema/github.graphql",
    query_path = "src/github/queries/dashboard.graphql",
    response_derives = "Debug"
)]
struct Dashboard;

/// A PR behind a Dashboard number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrRecord {
    pub number: i64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub draft: bool,
    pub created_at: String,
    pub merged_at: Option<String>,
}

/// Up to 100 records and how many GitHub counted in all; more than 100 means partial.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counted<T> {
    pub records: Vec<T>,
    pub total: i64,
}

impl<T> Counted<T> {
    pub fn partial(&self) -> bool {
        self.total > self.records.len() as i64
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metrics {
    pub repo: String,
    pub days: i64,
    /// The window's first day, `YYYY-MM-DD`.
    pub since: String,
    pub opened: Counted<PrRecord>,
    pub merged: Counted<PrRecord>,
    pub open: Counted<PrRecord>,
    pub waiting: Counted<PrRecord>,
    pub runs: Counted<Run>,
}

#[derive(Deserialize)]
struct RunPage {
    total_count: i64,
    workflow_runs: Vec<Run>,
}

/// `repo`'s PRs and runs over the last `days` days.
pub fn metrics(client: &Client, repo: &str, days: i64) -> Result<Metrics> {
    if !repo.contains('/') {
        return Err(anyhow!("Not an owner/name repository: {repo}"));
    }
    let since = crate::time::date_days_ago(days);
    let base = format!("repo:{repo} is:pr archived:false");
    let data = client.graphql::<Dashboard>(dashboard::Variables {
        opened: format!("{base} created:>={since}"),
        merged: format!("{base} merged:>={since}"),
        open: format!("{base} is:open"),
        waiting: format!("{base} is:open draft:false review:required"),
    })?;
    macro_rules! counted {
        ($field:ident, $node:ident) => {{
            let search = data.$field;
            let records = search
                .nodes
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|node| match node {
                    dashboard::$node::PullRequest(pr) => Some(record(pr)),
                    _ => None,
                })
                .collect();
            Counted {
                records,
                total: search.issue_count,
            }
        }};
    }
    let opened = counted!(opened, DashboardOpenedNodes);
    let merged = counted!(merged, DashboardMergedNodes);
    let open = counted!(open, DashboardOpenNodes);
    let waiting = counted!(waiting, DashboardWaitingNodes);
    let path = format!("repos/{repo}/actions/runs?per_page=100&created=%3E%3D{since}");
    let body = client.rest_text(&path, "application/vnd.github+json")?;
    let page: RunPage =
        serde_json::from_str(&body).context("GitHub returned unexpected Actions data")?;
    Ok(Metrics {
        repo: repo.to_owned(),
        days,
        since,
        opened,
        merged,
        open,
        waiting,
        runs: Counted {
            records: page.workflow_runs,
            total: page.total_count,
        },
    })
}

fn record(pr: dashboard::Rec) -> PrRecord {
    PrRecord {
        number: pr.number,
        title: pr.title,
        url: pr.url,
        author: pr.author.map_or_else(|| "ghost".into(), |a| a.login),
        draft: pr.is_draft,
        created_at: pr.created_at,
        merged_at: pr.merged_at,
    }
}
