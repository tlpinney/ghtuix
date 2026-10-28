# ghtuix

A rewrite of [ghtui](https://github.com/tlpinney/ghtui) as a small, plain Rust TUI for
GitHub pull requests and local Git. Same everyday workflow, a fraction of the code: ghtui
is about 382k product lines plus 390k test lines; ghtuix aims for well under 30k for v1.

ghtui's README, key bindings and screens are the behaviour reference. Where they and this
spec disagree, this spec wins.

## v1 features (in build order)

1. **Inbox** — PRs that involve you, in tabs: All, Created by me, Assigned to me, Needs my
   review, Merged (last 7 days). Table with repo#, title, author, age, review/check status.
   Preview pane beside the list: description, reviewers and commenters (bots included).
   `r` refresh; auto-refresh on an interval. Hidden repositories from config.
2. **PR view** — tabs Summary, Conversation, Files (diff), Checks. `←`/`b` back,
   `w` opens the PR in the browser.
3. **Writes** — comment, approve, request changes, reply to a review thread. Every write
   shows a preview of exactly what will be sent and needs an explicit confirm.
4. **Local Git** — History (commit graph), Working and Staged diffs for the current checkout.
   Read-only browsing never changes the working tree, index, refs or config.

Later (not v1): Actions runs and logs, local Git operations (stage, commit, branch),
search, notifications, dashboard.

## Architecture

- **One GitHub client** (`github/`). The token comes from `GH_TOKEN`/`GITHUB_TOKEN`, else
  once from `gh auth token --hostname <host>`; it stays in memory and is sent only to that
  host's API. Requests go over one pooled HTTPS agent (`ureq` + rustls); independent
  requests run in parallel on threads. One place maps HTTP and GraphQL errors.
- **Conforms to GitHub's spec.** GraphQL is the main API: each query lives in
  `src/github/queries/*.graphql` and is compiled by `graphql_client` against GitHub's
  published schema (`schema/github.graphql`, from docs.github.com), so a wrong field or
  type fails the build. The few REST-only calls (raw diffs, later Actions logs) are small
  typed functions. Refresh the schema with `scripts/update-schema.sh`.
- **One model per thing.** One `PullRequest`, one PR page, one form/confirm component for
  all writes. No parallel stacks for the same capability.
- **One action type.** Every control is a key bar entry or a row with an `Action`; keys and
  clicks dispatch the same `Action` through one `App::dispatch`.
- **Background work** runs on threads and reports back over one channel; the UI thread
  never blocks on the network.
- **Shared widgets**: key bar, table, pane block, theme roles. Views compose them.
- **Plain types and small modules.** Prefer an enum and a match over a trait hierarchy;
  no task-numbered modules; refactor instead of adding a parallel version.

## Testing

- Unit tests for parsing, state transitions and layout (render into a `TestBackend`).
- A few end-to-end checks of the real binary for the core flows, not one per feature.
- Every visible action has a test that its key and its click dispatch the same `Action`.

## Platforms

macOS and Linux. Windows users can run it under WSL2.
