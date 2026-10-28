//! GitHub Actions over REST: workflow runs, their jobs and steps, rerun and cancel.
//!
//! Reads are conditional: the caller passes the `ETag` and body it last saw, and an
//! unchanged list comes back as `304 Not Modified`, costing no rate limit.

use super::Client;
use crate::model::Checks;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Login {
    pub login: String,
}

/// A workflow run, as `GET /repos/{owner}/{repo}/actions/runs` returns it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: i64,
    pub name: Option<String>,
    pub display_title: String,
    pub status: Option<String>,
    pub conclusion: Option<String>,
    pub head_branch: Option<String>,
    pub event: String,
    pub run_number: i64,
    pub created_at: String,
    pub html_url: String,
    pub actor: Option<Login>,
}

/// A job of a run, with its steps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub started_at: Option<String>,
    pub html_url: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub number: i64,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
}

/// A body and its `ETag`, kept so the next read can be conditional.
pub type Saved = (String, String);

#[derive(Deserialize)]
struct RunList {
    workflow_runs: Vec<Run>,
}

#[derive(Deserialize)]
struct JobList {
    jobs: Vec<Job>,
}

/// The state a status and conclusion amount to, in the app's terms.
pub fn state(status: Option<&str>, conclusion: Option<&str>) -> Checks {
    match (status, conclusion) {
        (Some("completed"), Some("success" | "neutral" | "skipped")) => Checks::Passing,
        (Some("completed"), _) => Checks::Failing,
        _ => Checks::Pending,
    }
}

impl Run {
    pub fn state(&self) -> Checks {
        state(self.status.as_deref(), self.conclusion.as_deref())
    }

    /// `success`, `failure`, `in progress`, …
    pub fn outcome(&self) -> String {
        self.conclusion
            .as_deref()
            .or(self.status.as_deref())
            .unwrap_or("unknown")
            .replace('_', " ")
    }
}

impl Job {
    pub fn state(&self) -> Checks {
        state(Some(&self.status), self.conclusion.as_deref())
    }
}

/// The newest workflow runs of `repo`, using `saved` for a conditional read. Returns the
/// runs and, when GitHub sent a new body, that body and its `ETag` to save.
pub fn runs(
    client: &Client,
    repo: &str,
    saved: Option<Saved>,
) -> Result<(Vec<Run>, Option<Saved>)> {
    let (list, fresh): (RunList, _) = conditional(
        client,
        &format!("repos/{repo}/actions/runs?per_page=30"),
        saved,
    )?;
    Ok((list.workflow_runs, fresh))
}

/// Every job (and its steps) of one run, read the same way.
pub fn jobs(
    client: &Client,
    repo: &str,
    run_id: i64,
    saved: Option<Saved>,
) -> Result<(Vec<Job>, Option<Saved>)> {
    let path = format!("repos/{repo}/actions/runs/{run_id}/jobs?per_page=100");
    let (list, fresh): (JobList, _) = conditional(client, &path, saved)?;
    Ok((list.jobs, fresh))
}

pub(super) fn conditional<T: DeserializeOwned>(
    client: &Client,
    path: &str,
    saved: Option<Saved>,
) -> Result<(T, Option<Saved>)> {
    let reply = client.rest_conditional(path, saved.as_ref().map(|(etag, _)| etag.as_str()))?;
    let (body, fresh) = match (reply.body, saved) {
        (Some(body), _) => {
            let fresh = reply.etag.map(|etag| (etag, body.clone()));
            (body, fresh)
        }
        (None, Some((_, body))) => (body, None),
        (None, None) => anyhow::bail!("GitHub answered 304 without a saved copy"),
    };
    let parsed = serde_json::from_str(&body).context("GitHub returned unexpected data")?;
    Ok((parsed, fresh))
}

/// Re-runs the failed jobs of a run.
pub fn rerun_failed(client: &Client, repo: &str, run_id: i64) -> Result<()> {
    client.rest_post(&format!(
        "repos/{repo}/actions/runs/{run_id}/rerun-failed-jobs"
    ))
}

/// Cancels a run that is queued or in progress.
pub fn cancel(client: &Client, repo: &str, run_id: i64) -> Result<()> {
    client.rest_post(&format!("repos/{repo}/actions/runs/{run_id}/cancel"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_githubs_run_and_job_shapes() {
        let runs: RunList = serde_json::from_str(
            r#"{"total_count":1,"workflow_runs":[{"id":7,"name":"CI","display_title":"Fix",
            "status":"completed","conclusion":"failure","head_branch":"main","event":"push",
            "run_number":12,"created_at":"2026-10-27T10:00:00Z","html_url":"https://x",
            "actor":{"login":"alice","id":1},"extra":"ignored"}]}"#,
        )
        .unwrap();
        let run = &runs.workflow_runs[0];
        assert_eq!(run.state(), Checks::Failing);
        assert_eq!(run.outcome(), "failure");
        let jobs: JobList = serde_json::from_str(
            r#"{"jobs":[{"id":9,"name":"test","status":"in_progress","conclusion":null,
            "started_at":null,"html_url":null,"steps":[{"number":1,"name":"Set up",
            "status":"completed","conclusion":"success"}]}]}"#,
        )
        .unwrap();
        assert_eq!(jobs.jobs[0].state(), Checks::Pending);
        assert_eq!(jobs.jobs[0].steps[0].name, "Set up");
        assert_eq!(state(Some("completed"), Some("skipped")), Checks::Passing);
    }
}
