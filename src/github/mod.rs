//! The GitHub client: one token, one pooled HTTPS agent, typed GraphQL queries.
//!
//! Queries live in `queries/*.graphql` and are checked at compile time against GitHub's
//! published schema (`schema/github.graphql`).

use anyhow::{Context, Result, anyhow, bail};
use graphql_client::{GraphQLQuery, QueryBody, Response};
use std::process::Command;

pub mod viewer;

/// A signed-in connection to one GitHub host.
#[derive(Clone)]
pub struct Client {
    graphql_url: String,
    token: String,
    agent: ureq::Agent,
}

impl Client {
    /// Connects to `host` (`github.com` or a GitHub Enterprise Server) with the token from
    /// `GH_TOKEN`/`GITHUB_TOKEN`, else from `gh auth token`.
    pub fn connect(host: &str) -> Result<Self> {
        let token = token(host)?;
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .user_agent(concat!("ghtuix/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Ok(Self {
            graphql_url: graphql_url(host),
            token,
            agent,
        })
    }

    /// Runs one GraphQL query or mutation and returns its typed data.
    pub fn graphql<Q: GraphQLQuery>(&self, variables: Q::Variables) -> Result<Q::ResponseData> {
        let body: QueryBody<Q::Variables> = Q::build_query(variables);
        let mut response = self
            .agent
            .post(&self.graphql_url)
            .header("Authorization", &format!("Bearer {}", self.token))
            .send_json(&body)
            .with_context(|| format!("Could not reach {}", self.graphql_url))?;
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string()?;
        if !(200..300).contains(&status) {
            bail!("GitHub returned HTTP {status}: {}", http_message(&text));
        }
        let parsed: Response<Q::ResponseData> =
            serde_json::from_str(&text).context("GitHub returned an unexpected response")?;
        if let Some(errors) = parsed.errors.filter(|errors| !errors.is_empty()) {
            let messages: Vec<String> = errors.into_iter().map(|error| error.message).collect();
            bail!("GitHub: {}", messages.join("; "));
        }
        parsed
            .data
            .ok_or_else(|| anyhow!("GitHub returned no data"))
    }
}

fn graphql_url(host: &str) -> String {
    if host.eq_ignore_ascii_case("github.com") {
        "https://api.github.com/graphql".into()
    } else {
        format!("https://{host}/api/graphql")
    }
}

fn token(host: &str) -> Result<String> {
    for name in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(token) = std::env::var(name)
            && !token.trim().is_empty()
        {
            return Ok(token.trim().to_owned());
        }
    }
    let output = Command::new("gh")
        .args(["auth", "token", "--hostname", host])
        .output()
        .context("Could not run gh; install the GitHub CLI or set GH_TOKEN")?;
    if !output.status.success() {
        bail!("Not signed in to {host}: run `gh auth login --hostname {host}`");
    }
    let token = String::from_utf8(output.stdout)?.trim().to_owned();
    if token.is_empty() {
        bail!("gh returned an empty token for {host}");
    }
    Ok(token)
}

/// The `message` of a GitHub error body, or the body itself.
fn http_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("message")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphql_url_follows_the_host() {
        assert_eq!(graphql_url("github.com"), "https://api.github.com/graphql");
        assert_eq!(graphql_url("ghe.corp"), "https://ghe.corp/api/graphql");
    }

    #[test]
    fn http_message_prefers_githubs_message_field() {
        assert_eq!(
            http_message(r#"{"message":"Bad credentials"}"#),
            "Bad credentials"
        );
        assert_eq!(http_message("plain"), "plain");
    }
}
