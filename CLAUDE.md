# ghtuix — instructions for agents

Read SPEC.md first. It lists the v1 features, the architecture rules and the test policy.

## Rules

- **Keep it small and DRY.** Before adding a module, look for the one that should own the
  change and extend it. Never add a parallel version of an existing capability; refactor.
  No task or version numbers in names.
- **Plain idiomatic Rust.** Small functions, enums and matches, `anyhow::Result` at the
  edges, typed errors only where callers branch on them. Lines under 100 columns
  (`cargo fmt`). No `unsafe`. `cargo clippy --all-targets -- -D warnings` stays clean.
- **One dispatch path.** Keys and clicks produce the same `Action` and go through
  `App::dispatch`.
- **GitHub API through `src/github/`.** Use `gh` only to obtain the token. New GraphQL
  queries go in `src/github/queries/` and must compile against `schema/github.graphql`;
  don't hand-write JSON for something the schema types. Read-only browsing never writes;
  every write shows a preview and needs an explicit confirm.
- **Tests:** unit tests next to the code (`#[cfg(test)] mod tests`). Run `cargo test`
  before committing. Keep the suite fast (seconds, not minutes).

## Workflow

- Commit and push straight to `main` (no PRs). Run `cargo fmt`, `cargo clippy --all-targets
  -- -D warnings` and `cargo test` first.
- Commit identity: the repository's default (Travis Pinney).
