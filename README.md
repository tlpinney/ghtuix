# ghtuix

A small, fast terminal UI for GitHub pull requests, Actions and your local Git checkout.
A plain-Rust rewrite of [ghtui](https://github.com/tlpinney/ghtui); see [SPEC.md](SPEC.md)
for the design.

## Install

Requires Rust 1.85+ (edition 2024), `git`, and the [GitHub CLI](https://cli.github.com)
signed in (`gh auth login`). ghtuix only asks `gh` for your token.

```sh
cargo install --path .
```

Run it inside a Git checkout to see that repository's history, changes and Actions:

```sh
cd ~/code/my-project
ghtuix
```

## Views and keys

Every control on screen is clickable and shows its key. Writes always show a preview of
exactly what will happen and wait for **Enter** to confirm; **Esc** steps back.

**Everywhere:** `1` Inbox, `2` Local, `3` Actions, `4` Notifications · `r` refresh · `q` quit ·
`↑/↓` or `j/k` move · PgUp/PgDn and the wheel scroll.

### 1 Inbox

| Key | Action |
| --- | --- |
| `a` `c` `m` `v` `M` | All · Created by me · Assigned to me · Needs my review · Merged (7 days) |
| `Enter` / click a selected row | Open the PR |
| `w` | Open the PR in the browser |

The preview (120+ columns wide) shows the description and everyone involved: reviewers
with where they stand, and commenters, bots included.

### A pull request

| Key | Action |
| --- | --- |
| `o` `c` `f` `k` | Summary · Conversation · Files · Checks |
| `n` `A` `X` | Comment · Approve · Request changes |
| `i` (Files) | Comment on the diff line under the cursor |
| `R` `Z` (Conversation) | Reply in the selected review thread · resolve or unresolve it |
| `Enter` (Checks) | Open the selected GitHub Actions job's log (`>` `<` jump between failures) |
| `M` | Merge (`m` merge commit, `s` squash, `r` rebase); refused if the head moved |
| `p` | PR actions: close or reopen, draft or ready for review, edit the title, labels, request reviewers (type to filter, `Space` checks) |
| `w` · `b`/`←`/`Esc` | Open on the web · back to the Inbox |

### 2 Local

| Key | Action |
| --- | --- |
| `g` `w` `i` | History (git's graph of every branch) · Working changes · Staged changes |
| `Enter` (History) | Show the selected commit |
| `+` / `-` | Stage / unstage the selected file |
| `K` | Commit what is staged (preview shows branch, author, files and message) |

Browsing never changes the checkout: git runs with `--no-optional-locks`, so even
`git status` does not touch the index.

### 3 Actions

| Key | Action |
| --- | --- |
| `Enter` | Open the selected run (its jobs and steps), then a job's log |
| `R` `C` | Re-run the failed jobs · cancel the run |
| `w` | Open the run on GitHub |

### 4 Notifications

| Key | Action |
| --- | --- |
| `Enter` / click a selected row | Open a PR notification in the PR view (Back returns here); others open on the web |
| `x` `D` | Mark the notification read · mark it done (it leaves GitHub's inbox) |
| `a` / `u` | Show all notifications · only unread ones |
| `w` | Open on GitHub |

## Configuration

`~/.config/ghtuix/config.toml` (or `$XDG_CONFIG_HOME/ghtuix/config.toml`):

```toml
host = "github.com"                       # or a GitHub Enterprise Server host
excluded_repositories = ["owner/noisy"]   # never shown in the Inbox
```

## Speed and GitHub's limits

- The Inbox is one GraphQL request (about 6 of the 5,000 points an hour). It appears from
  the local cache at start, then refreshes every minute.
- A PR page is one request, skipped entirely when the PR has not changed since it was
  cached; diffs between two commits and finished job logs are cached for good.
- Actions and notification lists are conditional reads: an unchanged list costs nothing
  (`304`).
- When GitHub's budget runs low, automatic refresh slows down and then pauses until the
  reset; the status line says so.

The cache lives in `~/.cache/ghtuix/cache.sqlite3`, keyed by host and account. Delete it
to start fresh.
