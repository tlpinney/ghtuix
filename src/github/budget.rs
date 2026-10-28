//! GitHub's rate limits, as every response reports them, and the client's request gate.
//!
//! Each response carries `x-ratelimit-remaining`/`-limit`/`-reset` for its resource
//! (`x-ratelimit-resource`): GraphQL and REST have separate budgets. A 403 or 429 with
//! nothing remaining blocks that resource until its reset; one with `retry-after` (a
//! secondary limit) blocks every request until GitHub says to resume. At most
//! [`MAX_IN_FLIGHT`] requests run at once; the rest wait.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Concurrent requests per client (GitHub allows 100; a TUI never needs more than a few).
pub const MAX_IN_FLIGHT: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    pub limit: u32,
    pub remaining: u32,
    /// When the hourly budget resets (Unix seconds).
    pub reset_at: i64,
    /// No request is sent before this time (Unix seconds).
    pub blocked_until: Option<i64>,
}

impl Budget {
    /// Updates from one response's status and headers (`header` looks one up by name).
    pub fn observe(&mut self, status: u16, header: impl Fn(&str) -> Option<String>, now: i64) {
        let number = |name: &str| header(name)?.trim().parse::<i64>().ok();
        if let (Some(limit), Some(remaining)) =
            (number("x-ratelimit-limit"), number("x-ratelimit-remaining"))
        {
            self.limit = limit.max(0) as u32;
            self.remaining = remaining.max(0) as u32;
        }
        if let Some(reset) = number("x-ratelimit-reset") {
            self.reset_at = reset;
        }
        if matches!(status, 403 | 429) {
            if let Some(after) = number("retry-after") {
                self.blocked_until = Some(now + after.max(1));
            } else if self.limit > 0 && self.remaining == 0 {
                self.blocked_until = Some(self.reset_at.max(now + 1));
            }
        }
    }

    pub fn blocked(&self, now: i64) -> Option<i64> {
        self.blocked_until.filter(|until| *until > now)
    }

    fn fraction_left(&self) -> f64 {
        if self.limit == 0 {
            1.0
        } else {
            f64::from(self.remaining) / f64::from(self.limit)
        }
    }

    /// How long automatic refresh waits: `normal` while the budget is healthy, 5 minutes
    /// below 20 % left, and until the reset below 5 % or while blocked.
    pub fn refresh_interval(&self, normal: Duration, now: i64) -> Duration {
        let until = |at: i64| Duration::from_secs((at - now).max(1) as u64);
        if let Some(blocked) = self.blocked(now) {
            until(blocked)
        } else if self.fraction_left() < 0.05 && self.reset_at > now {
            until(self.reset_at)
        } else if self.fraction_left() < 0.2 {
            normal.max(Duration::from_secs(300))
        } else {
            normal
        }
    }

    /// A status note when the budget needs attention, else `None`.
    pub fn note(&self, now: i64) -> Option<String> {
        if let Some(until) = self.blocked(now) {
            return Some(format!(
                "GitHub rate limit reached • requests resume in {}",
                wait(until - now)
            ));
        }
        (self.fraction_left() < 0.2).then(|| {
            format!(
                "GitHub API budget low: {}/{} left, resets in {} • auto-refresh slowed",
                self.remaining,
                self.limit,
                wait(self.reset_at - now)
            )
        })
    }
}

fn wait(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 120 {
        format!("{seconds}s")
    } else {
        format!("{}m", seconds / 60)
    }
}

/// Which of GitHub's budgets a request spends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    Graphql,
    Rest,
}

/// The GraphQL and REST budgets, and the in-flight count.
#[derive(Debug, Default)]
struct State {
    graphql: Budget,
    rest: Budget,
    in_flight: usize,
}

impl State {
    fn budget(&mut self, resource: Resource) -> &mut Budget {
        match resource {
            Resource::Graphql => &mut self.graphql,
            Resource::Rest => &mut self.rest,
        }
    }
}

/// Shared by every clone of a client: the budgets and the in-flight count.
#[derive(Debug, Default)]
pub struct Gate {
    state: Mutex<State>,
    freed: Condvar,
    /// When the last write finished; held for the whole of each write.
    last_write: Mutex<Option<std::time::Instant>>,
}

/// The pause GitHub asks for between writes.
pub const WRITE_SPACING: Duration = Duration::from_secs(1);

impl Gate {
    /// Waits for a free slot; refuses at once while GitHub has asked to wait.
    pub fn enter(&self, resource: Resource, now: i64) -> Result<Slot<'_>, String> {
        let mut state = self.state.lock().unwrap();
        if let Some(until) = state.budget(resource).blocked(now) {
            return Err(format!(
                "GitHub rate limit reached • not sent; requests resume in {}",
                wait(until - now)
            ));
        }
        while state.in_flight >= MAX_IN_FLIGHT {
            state = self.freed.wait(state).unwrap();
        }
        state.in_flight += 1;
        Ok(Slot { gate: self })
    }

    /// Waits for the previous write to finish and for [`WRITE_SPACING`] to pass; the turn
    /// lasts until the returned guard drops.
    pub fn write_turn(&self) -> WriteTurn<'_> {
        let last = self.last_write.lock().unwrap();
        if let Some(at) = *last {
            std::thread::sleep(WRITE_SPACING.saturating_sub(at.elapsed()));
        }
        WriteTurn { last }
    }

    /// The budget that needs attention first: a blocked one, else the one with the
    /// smaller share left. Automatic refresh and the status note follow it.
    pub fn budget(&self, now: i64) -> Budget {
        let state = self.state.lock().unwrap();
        let (a, b) = (state.graphql, state.rest);
        match (a.blocked(now), b.blocked(now)) {
            (Some(x), Some(y)) => {
                if x >= y {
                    a
                } else {
                    b
                }
            }
            (Some(_), None) => a,
            (None, Some(_)) => b,
            (None, None) if b.fraction_left() < a.fraction_left() => b,
            (None, None) => a,
        }
    }

    /// Records a response of a request made for `resource` (GitHub's own
    /// `x-ratelimit-resource` header decides when present).
    pub fn observe(
        &self,
        resource: Resource,
        status: u16,
        header: impl Fn(&str) -> Option<String>,
        now: i64,
    ) {
        let resource = match header("x-ratelimit-resource").as_deref() {
            Some("graphql") => Resource::Graphql,
            Some(_) => Resource::Rest,
            None => resource,
        };
        let mut state = self.state.lock().unwrap();
        state.budget(resource).observe(status, &header, now);
        // A secondary limit (`retry-after`) holds every request, not just this kind.
        let paused = state.budget(resource).blocked_until;
        if matches!(status, 403 | 429) && header("retry-after").is_some() {
            let other = match resource {
                Resource::Graphql => Resource::Rest,
                Resource::Rest => Resource::Graphql,
            };
            state.budget(other).blocked_until = paused;
        }
    }
}

/// One write's turn; records when it ended.
pub struct WriteTurn<'a> {
    last: std::sync::MutexGuard<'a, Option<std::time::Instant>>,
}

impl Drop for WriteTurn<'_> {
    fn drop(&mut self) {
        *self.last = Some(std::time::Instant::now());
    }
}

/// One request's place in the gate, released when dropped.
#[derive(Debug)]
pub struct Slot<'a> {
    gate: &'a Gate,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.gate.state.lock().unwrap().in_flight -= 1;
        self.gate.freed.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn a_healthy_budget_changes_nothing() {
        let mut budget = Budget::default();
        let ok = [
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-remaining", "4900"),
        ];
        budget.observe(200, headers(&ok), 0);
        assert_eq!(budget.remaining, 4900);
        assert_eq!(budget.note(0), None);
        let normal = Duration::from_secs(60);
        assert_eq!(budget.refresh_interval(normal, 0), normal);
    }

    #[test]
    fn a_low_budget_slows_refresh_and_an_empty_one_waits_for_the_reset() {
        let mut budget = Budget::default();
        let low = [
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-remaining", "600"),
            ("x-ratelimit-reset", "3600"),
        ];
        budget.observe(200, headers(&low), 0);
        let normal = Duration::from_secs(60);
        assert_eq!(budget.refresh_interval(normal, 0), Duration::from_secs(300));
        assert!(budget.note(0).unwrap().contains("600/5000"));
        let empty = [
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", "3600"),
        ];
        budget.observe(403, headers(&empty), 0);
        assert_eq!(budget.blocked(0), Some(3600));
        assert_eq!(
            budget.refresh_interval(normal, 0),
            Duration::from_secs(3600)
        );
    }

    #[test]
    fn retry_after_blocks_the_gate_until_it_passes() {
        let gate = Gate::default();
        gate.observe(Resource::Rest, 429, headers(&[("retry-after", "30")]), 100);
        assert!(
            gate.enter(Resource::Graphql, 110)
                .unwrap_err()
                .contains("resume in 20s")
        );
        assert!(gate.enter(Resource::Graphql, 131).is_ok());
    }

    #[test]
    fn writes_are_spaced_at_least_a_second_apart() {
        let gate = Gate::default();
        drop(gate.write_turn());
        let start = std::time::Instant::now();
        drop(gate.write_turn());
        assert!(start.elapsed() >= WRITE_SPACING - Duration::from_millis(5));
    }

    #[test]
    fn the_gate_holds_at_most_max_in_flight() {
        let gate = std::sync::Arc::new(Gate::default());
        let slots: Vec<_> = (0..MAX_IN_FLIGHT)
            .map(|_| gate.enter(Resource::Graphql, 0).unwrap())
            .collect();
        let waiter = {
            let gate = gate.clone();
            std::thread::spawn(move || gate.enter(Resource::Graphql, 0).map(|_| ()).is_ok())
        };
        std::thread::sleep(Duration::from_millis(50));
        assert!(!waiter.is_finished(), "the fifth request waits");
        drop(slots);
        assert!(waiter.join().unwrap());
    }

    #[test]
    fn graphql_and_rest_keep_their_own_budgets() {
        let gate = Gate::default();
        let low = [
            ("x-ratelimit-resource", "graphql"),
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-remaining", "100"),
            ("x-ratelimit-reset", "500"),
        ];
        let healthy = [
            ("x-ratelimit-resource", "core"),
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-remaining", "4900"),
        ];
        gate.observe(Resource::Graphql, 200, headers(&low), 0);
        gate.observe(Resource::Rest, 200, headers(&healthy), 0);
        assert_eq!(
            gate.budget(0).remaining,
            100,
            "a healthy REST read hides nothing"
        );
        // REST running out blocks REST, not GraphQL.
        let empty = [
            ("x-ratelimit-resource", "core"),
            ("x-ratelimit-limit", "5000"),
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", "900"),
        ];
        gate.observe(Resource::Rest, 403, headers(&empty), 10);
        assert!(gate.enter(Resource::Rest, 20).is_err());
        assert!(gate.enter(Resource::Graphql, 20).is_ok());
    }
}
