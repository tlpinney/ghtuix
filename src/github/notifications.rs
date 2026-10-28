//! The signed-in account's notifications over REST: read conditionally, marked read or done.

use super::Client;
use super::actions::Saved;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A notification thread, as `GET /notifications` returns it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub unread: bool,
    /// Why it came: `review_requested`, `mention`, `ci_activity`, …
    pub reason: String,
    pub updated_at: String,
    pub subject: Subject,
    pub repository: Repository,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subject {
    pub title: String,
    /// The subject's API URL (`…/repos/o/r/pulls/5`); missing for some kinds.
    pub url: Option<String>,
    /// `PullRequest`, `Issue`, `Release`, `CheckSuite`, `Discussion`, …
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    pub full_name: String,
    pub html_url: String,
}

impl Notification {
    /// The PR's number, when the notification is about a pull request.
    pub fn pr_number(&self) -> Option<i64> {
        if self.subject.kind != "PullRequest" {
            return None;
        }
        self.subject
            .url
            .as_deref()?
            .rsplit('/')
            .next()?
            .parse()
            .ok()
    }

    /// Where the subject lives on the web: the API URL's path mapped onto the
    /// repository's page, or the repository itself when there is none.
    pub fn web_url(&self) -> String {
        let base = &self.repository.html_url;
        let tail = self
            .subject
            .url
            .as_deref()
            .and_then(|url| url.split_once(&format!("/repos/{}/", self.repository.full_name)))
            .map(|(_, tail)| tail);
        match tail.and_then(|tail| tail.split_once('/')) {
            Some(("pulls", rest)) => format!("{base}/pull/{rest}"),
            Some(("commits", rest)) => format!("{base}/commit/{rest}"),
            Some(("issues", rest)) => format!("{base}/issues/{rest}"),
            Some(("discussions", rest)) => format!("{base}/discussions/{rest}"),
            _ => base.clone(),
        }
    }
}

/// The newest notifications (unread only, unless `all`), read conditionally.
pub fn list(
    client: &Client,
    all: bool,
    saved: Option<Saved>,
) -> Result<(Vec<Notification>, Option<Saved>)> {
    super::actions::conditional(
        client,
        &format!("notifications?per_page=50&all={all}"),
        saved,
    )
}

/// Marks a thread read; it stays in the list of all notifications.
pub fn mark_read(client: &Client, id: &str) -> Result<()> {
    client.rest_write("PATCH", &format!("notifications/threads/{id}"))
}

/// Marks a thread done, removing it from the inbox until something new happens.
pub fn mark_done(client: &Client, id: &str) -> Result<()> {
    client.rest_write("DELETE", &format!("notifications/threads/{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(kind: &str, url: Option<&str>) -> Notification {
        Notification {
            id: "1".into(),
            unread: true,
            reason: "review_requested".into(),
            updated_at: "2026-10-27T10:00:00Z".into(),
            subject: Subject {
                title: "Fix it".into(),
                url: url.map(str::to_owned),
                kind: kind.into(),
            },
            repository: Repository {
                full_name: "o/a".into(),
                html_url: "https://github.com/o/a".into(),
            },
        }
    }

    #[test]
    fn subjects_map_to_their_pages() {
        let pr = notification(
            "PullRequest",
            Some("https://api.github.com/repos/o/a/pulls/5"),
        );
        assert_eq!(pr.pr_number(), Some(5));
        assert_eq!(pr.web_url(), "https://github.com/o/a/pull/5");
        let issue = notification(
            "Issue",
            Some("https://ghe.example/api/v3/repos/o/a/issues/7"),
        );
        assert_eq!(issue.pr_number(), None);
        assert_eq!(issue.web_url(), "https://github.com/o/a/issues/7");
        let suite = notification("CheckSuite", None);
        assert_eq!(suite.web_url(), "https://github.com/o/a");
    }

    #[test]
    fn parses_githubs_shape() {
        let list: Vec<Notification> = serde_json::from_str(
            r#"[{"id":"9","unread":false,"reason":"mention","updated_at":"2026-10-27T10:00:00Z",
            "last_read_at":null,"subject":{"title":"Hi","url":null,"latest_comment_url":null,
            "type":"Release"},"repository":{"id":1,"full_name":"o/a",
            "html_url":"https://github.com/o/a"},"url":"x","subscription_url":"y"}]"#,
        )
        .unwrap();
        assert_eq!(list[0].subject.kind, "Release");
    }
}
