# zad github

> Runtime verbs for the GitHub service — issues, pull requests, files,
> code search, and workflow runs, scoped per repository and
> organization.

## Synopsis

```
zad github <VERB> [SUBVERB] [OPTIONS]
```

## Description

`zad github` operates the GitHub service at runtime. The project must
already have GitHub enabled (`zad service enable github`) and a
Personal Access Token registered in either scope — runtime commands
resolve the effective configuration with local winning over global,
then load the matching PAT from the OS keychain and pass it to the
`gh` CLI via `GH_TOKEN`.

| Verb | Description |
|---|---|
| `issue list`     | List issues in a repo. |
| `issue view`     | Fetch one issue's body and comments. |
| `issue create`   | Open a new issue. |
| `issue comment`  | Post a comment on an issue. |
| `issue close`    | Close an issue (optionally with a final comment). |
| `pr list`        | List pull requests in a repo. |
| `pr view`        | Fetch one PR's body, reviews, and conversation. |
| `pr diff`        | Print a PR's unified diff. |
| `pr create`      | Open a new pull request. |
| `pr comment`     | Post a conversation comment on a PR. |
| `pr review`      | File a review (approve, request changes, or comment). |
| `pr merge`       | Merge a PR (squash, merge, or rebase). |
| `pr checks`      | Show CI check summary for a PR. |
| `repo view`      | Show repository metadata. |
| `file view`      | Print the contents of a file at a given ref. |
| `code search`    | Full-text code search via GitHub's search API. |
| `run list`       | List recent workflow runs. |
| `run view`       | Show details (and optional logs) for a run. |
| `permissions`    | Inspect, scaffold, or dry-run the permissions policy. |

Read verbs emit `gh`'s `--json` output when `--json` is passed so the
envelope is stable for piped consumers. Mutating verbs support
`--dry-run`, which records a preview (human summary + structured
payload) without spawning `gh`.

## Repository addressing

Every verb accepts `--repo owner/name`. If `default_repo` is set in the
effective config, `--repo` can be omitted. Verbs that search across an
organization (`code search`) also accept `--org` or fall back to
`default_owner`.

## Requirements

The `gh` CLI must be on `PATH`. Install it from
<https://cli.github.com/> (e.g. `brew install gh`, `apt install gh`,
`scoop install gh`). `zad github` does **not** read your personal
`gh auth login` state — it passes the zad-managed PAT via `GH_TOKEN`
so per-project credentials work as expected.

## Scope enforcement

Every runtime verb checks the required scope against the `scopes`
array in the effective credentials file **before** any subprocess
spawn. Missing the scope returns a `scope denied` error that names
the exact file path to edit. The mapping is:

| Verb | Required scope |
|---|---|
| `issue list`, `issue view`     | `issues.read` |
| `issue create/comment/close`   | `issues.write` |
| `pr list/view/diff`            | `pulls.read` |
| `pr create/comment/review/merge` | `pulls.write` |
| `pr checks`, `run list/view`   | `checks.read` |
| `repo view`, `file view`       | `repo.read` |
| `code search`                  | `search` |

Default scope set at `zad service create github`: `repo.read`,
`issues.read`, `pulls.read`, `checks.read`, `search`. Write scopes
(`issues.write`, `pulls.write`) are opt-in — pass them to `--scopes`
during create, or re-run create with `--force`.

## Permissions (second layer)

Scope is the coarse gate — "is this family of operations enabled?".
**Permissions** are the fine gate — "is *this* call (to this repo, at
this time, with this body) allowed?". They live in an optional TOML
file at:

- Global: `~/.zad/services/github/permissions.toml`
- Local:  `~/.zad/projects/<slug>/services/github/permissions.toml`

Both files apply — a call must pass every file that exists. Missing
files contribute no restrictions. `docs/configuration.md` documents
the full schema (`repos` and `orgs` allow/deny lists, content rules,
UTC time windows, per-function blocks). The mapping from verb to
function block is:

| Verb | Permissions block | Matches against |
|---|---|---|
| `issue list`    | `[issue_list]`    | `repos` |
| `issue view`    | `[issue_view]`    | `repos` |
| `issue create`  | `[issue_create]`  | `repos`; body against `content` |
| `issue comment` | `[issue_comment]` | `repos`; body against `content` |
| `issue close`   | `[issue_close]`   | `repos`; optional comment against `content` |
| `pr list`       | `[pr_list]`       | `repos` |
| `pr view`       | `[pr_view]`       | `repos` |
| `pr diff`       | `[pr_diff]`       | `repos` |
| `pr create`     | `[pr_create]`     | `repos`; title+body against `content` |
| `pr comment`    | `[pr_comment]`    | `repos`; body against `content` |
| `pr review`     | `[pr_review]`     | `repos`; body against `content` |
| `pr merge`      | `[pr_merge]`      | `repos` (default-deny in starter) |
| `pr checks`     | `[pr_checks]`     | `repos` |
| `repo view`     | `[repo_view]`     | `repos` |
| `file view`     | `[file_view]`     | `repos` |
| `code search`   | `[code_search]`   | `repos`, `orgs` |
| `run list`      | `[run_list]`      | `repos` |
| `run view`      | `[run_view]`      | `repos` |

Permission violations surface with a `permission denied` error that
names the function, the reason, and the exact file path to edit — the
same shape as the scope-denied error.

## Examples

Scaffold a starter policy, then list the open issues on `myorg/webapp`:

```
zad github permissions init
zad github issue list --repo myorg/webapp --state open --limit 10
```

Comment on an issue (respects `[issue_comment]` repo allow-list):

```
zad github issue comment 42 --repo myorg/webapp \
  --body "Triaging; I'll pick this up tomorrow."
```

Dry-run a merge to preview the call without invoking `gh`:

```
zad github pr merge 17 --repo myorg/webapp --squash --dry-run
```

Cross-repo code search limited to one org:

```
zad github code search 'fn main extension:rs' --org myorg --limit 20
```

View a specific file at a ref:

```
zad github file view --repo myorg/webapp --path src/main.rs --ref v1.2.0
```

## See also

- `zad man service` — lifecycle commands (`create`, `enable`, `show`,
  `delete`).
- `zad man docs` / `zad docs configuration` — full configuration and
  permissions schema.
- `zad man permissions` — the staged-commit workflow every service
  ships for mutating permissions files safely.
