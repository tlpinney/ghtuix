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

**Everywhere:** `1` Inbox, `2` Local, `3` Actions, `4` Notifications, `5` Search, `6` Dashboard · `r` refresh · `T` theme · `q` quit ·
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
| `g` `w` `i` `B` `S` | History (git's graph of every branch) · Working changes · Staged changes · Branches · Stash |
| `Enter` (History) | Show the selected commit |
| `+` / `-` | Stage / unstage the selected file |
| `Enter` (Working, Staged) or click a changed line | Put a cursor in the file's diff; `↑/↓` step between changed lines |
| `+` / `-` · `L` (in the diff) | Stage / unstage the hunk under the cursor · just that line (`b` back to the files) |
| `K` | Commit what is staged (preview shows branch, author, files and message) |

**Branches:** `Enter` switches to the selected branch, `N` creates one at the current
commit, `D` deletes a fully merged one (`git branch -d`), and `R` rebases the current
branch onto it; its preview lists every commit it will replay. A rebase that stops on a
conflict shows a banner with `C` Continue and `X` Abort.
**Stash:** `Z` stashes every change, untracked files too, with an optional message;
`Enter` shows a stash, `A` applies it, `P` pops it and `X` drops it.
Each of these previews the exact git command and runs only after **Enter**.

A hunk or line is staged with `git apply --cached` (`--reverse` to unstage), so the
working tree is never touched; if the file changed since it was read, git refuses the
patch and the error shows. Browsing never changes the checkout: git runs with `--no-optional-locks`, so even
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

### 5 Search

Type [GitHub search syntax](https://docs.github.com/search-github/searching-on-github/searching-issues-and-pull-requests)
(`is:pr is:open review-requested:@me`, `repo:owner/name label:bug`) and press **Enter**;
**Esc** leaves the query. One request returns the first 50 issues and PRs.

| Key | Action |
| --- | --- |
| `Enter` / click a selected row | Open a PR in the PR view (Back returns here); an issue on the web |
| `/` or click the query | Edit the search |
| `w` | Open on GitHub |

### 6 Dashboard

PR and CI numbers for the checkout's repository over the last 7 or 30 days: PRs opened,
merged (with the median time to merge), open, waiting for review, workflow runs (pass
rate), failed runs and run time. Each card lists the records behind its number. When
GitHub has more than the 100 newest records read, the card says the figure is partial;
with nothing to measure it says so instead of showing zero. One GraphQL request and one
Actions read.

| Key | Action |
| --- | --- |
| `Enter` / click a selected card | List its records; `Enter` opens a PR in the PR view (Back returns here) or a run on the web |
| `d` | Switch between 7 and 30 days |
| `b`/`←`/`Esc` · `w` | Back to the cards · open the record on GitHub |

## Configuration

`~/.config/ghtuix/config.toml` (or `$XDG_CONFIG_HOME/ghtuix/config.toml`):

```toml
host = "github.com"                       # or a GitHub Enterprise Server host
excluded_repositories = ["owner/noisy"]   # never shown in the Inbox
theme = "catppuccin-mocha"                # the default; see below
```

**Themes:** `catppuccin-mocha` (default), `catppuccin-latte` (light), `tokyo-night`,
`dracula`, `gruvbox-dark`, `nord`, and `terminal` (your terminal's own colours and
background). `T` previews them live; `theme = "…"` in the config keeps one.
`NO_COLOR=1` turns colour off.

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
