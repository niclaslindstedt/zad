# zad telegram

> Runtime verbs for the Telegram service — send, read, list chats,
> discover and curate a name → chat_id directory.

## Synopsis

```
zad telegram <VERB> [OPTIONS]
```

## Description

`zad telegram` operates the Telegram service at runtime. The project
must already have Telegram enabled (`zad service enable telegram`) and
valid credentials registered in either scope — runtime commands
resolve the effective configuration with local winning over global,
then load the matching bot token from the OS keychain.

| Verb | Description |
|---|---|
| `send`        | Send a message to a chat (private/group/supergroup/channel). |
| `read`        | Fetch recent messages the bot has buffered for a chat. |
| `chats`       | List chats the bot has seen (local directory plus recent updates). |
| `discover`    | Poll the Bot API for recent updates and cache chat aliases. |
| `directory`   | Inspect or hand-edit the name → chat_id directory. |
| `permissions` | Inspect, scaffold, or dry-run the per-project permissions policy. |

Every verb supports `--json` to emit machine-readable output instead
of the human-readable default.

## Chat addressing

Telegram addresses every target — DMs, groups, supergroups, and
channels — through a single signed `chat_id`:

- **Private chats**: positive integer, equal to the user's user_id.
- **Groups and supergroups**: negative integer (e.g.
  `-1001234567890` for supergroups — the `-100` prefix is part of the
  ID, not a sign convention).
- **Public channels / supergroups**: also addressable by `@username`
  from the chat's public link.

Every runtime verb also accepts a **directory alias** — a short name
from this project's
`~/.zad/projects/<slug>/services/telegram/directory.toml`. Add aliases
with `zad telegram directory set <name> <id>`.

## Scope enforcement

Every runtime verb checks the required scope against the `scopes`
array in the effective credentials file **before** any network call.
Missing the scope returns a `scope denied` error that names the exact
file path to edit. The mapping is:

| Verb | Required scope |
|---|---|
| `send`        | `messages.send` |
| `read`        | `messages.read` |
| `chats`, `discover` | `chats` |
| `directory`, `permissions` | none (local state only) |

See `docs/configuration.md` for the full scope list and for the
local-vs-global precedence rules.

## Permissions (second layer)

Scope is the coarse gate — "is this family of operations enabled?".
**Permissions** are the fine gate — "is *this* call (to this chat, at
this time, with this content) allowed?". They live in an optional
TOML file at:

- Global: `~/.zad/services/telegram/permissions.toml`
- Local:  `~/.zad/projects/<slug>/services/telegram/permissions.toml`

Both files apply — a call must pass every file that exists. Missing
files contribute no restrictions. The `docs/configuration.md` file
documents the full schema (allow/deny globs and regex, denied content
words and patterns, UTC time windows, per-function blocks). The
mapping from verb to function block is:

| Verb | Permissions block | Matches against |
|---|---|---|
| `send`     | `[send]`     | `chats` (for `--chat`); body against `content` |
| `read`     | `[read]`     | `chats` |
| `chats`    | `[chats]`    | `chats` |
| `discover` | `[discover]` | `chats` — denied chats are skipped in the walk |

Permission violations surface with a `permission denied` error that
names the function, the reason, and the exact file path to edit — the
same shape as the scope-denied error.

## Name resolution

`--chat` accepts any of:

- a numeric `chat_id` (positive or negative),
- a public `@username` (looked up as a directory key after stripping the `@`),
- a name from this project's directory.

When the name is unknown, the error message prints the exact
`zad telegram directory set …` command that would map it.

## `zad telegram send`

```
zad telegram send [--chat <ID|@USERNAME|NAME>] [--stdin] [BODY]
```

Post a message. The body is taken from the positional argument, or
from standard input when `--stdin` is set. Bodies longer than
Telegram's 4096-codepoint hard limit are rejected locally (no
round-trip).

| Flag | Type | Default | Description |
|---|---|---|---|
| `--chat <id\|@username\|name>` | chat_id \| `@username` \| directory name | `default_chat` from the effective config | Destination chat. |
| `--stdin` | bool | `false` | Read the body from standard input instead of a positional argument. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |
| `--dry-run` | bool | `false` | Preview the outgoing call without contacting the Bot API — prints the payload as JSON on stdout and makes no network request. Scope and permission checks still run; no bot token is loaded. |

## `zad telegram read`

```
zad telegram read --chat <ID|@USERNAME|NAME> [--limit N]
```

Fetch up to `--limit` recent messages the bot has buffered for
`--chat`. The Bot API's update stream is **forward-only** — only
messages observed since the bot's previous `getUpdates` call are
returned, so `read` is best used in a long-lived workflow where
updates accumulate between invocations. The empty case prints a hint
that points to the caveat instead of silently succeeding.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--chat <id\|@username\|name>` | chat_id \| `@username` \| directory name | — | Chat to filter updates by. |
| `--limit <n>` | integer | `20` | Maximum number of messages to return (1–100). |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

## `zad telegram chats`

```
zad telegram chats [--json]
```

List chats the bot has learned about: the project's directory cache
plus any chats observed in a short `getUpdates` poll. Telegram has no
"list every chat I'm in" endpoint, so this is a **best-effort**
surface by design. The `SOURCE` column marks each row as
`directory` (only in the local cache) or `observed` (seen in the
recent updates batch).

## `zad telegram discover`

```
zad telegram discover [--json]
```

Poll the Bot API for recent updates and upsert every chat seen into
this project's `directory.toml`. Hand-authored entries are preserved.
Chats that the `[discover]` permission block denies are silently
skipped, mirroring the best-effort shape of every `discover` verb.
Safe to re-run.

## `zad telegram directory`

```
zad telegram directory                        # list
zad telegram directory set    <name> <id>     # upsert a mapping
zad telegram directory remove <name>          # delete a mapping
zad telegram directory clear  --force         # wipe the file
```

`<id>` is a signed decimal integer (groups and supergroups use
negative IDs). `remove` is idempotent.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--force` | bool | `false` | Required by `clear` to confirm wiping the directory. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

## `zad telegram permissions`

```
zad telegram permissions                         # show (same as `show`)
zad telegram permissions show
zad telegram permissions path
zad telegram permissions init  [--local] [--force]
zad telegram permissions check --function <name> [--chat <id|name>] [--body <text>]
```

- `show` — print both candidate file paths plus the body of whichever
  files exist.
- `path` — print only the two candidate paths, one per line.
- `init` — write a starter policy. Defaults to the global scope; pass
  `--local` to target `~/.zad/projects/<slug>/services/telegram/`.
  The template denies admin-like chats.
- `check` — dry-run a proposed action against the effective policy.
  Exits 0 on allow, 1 on deny with the reason and the config path
  printed.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--function <name>` | string | — | One of `send`, `read`, `chats`, `discover`. |
| `--chat <id\|name>` | chat_id \| directory name | — | Target chat for chat-scoped functions. |
| `--body <text>` | string | — | Message body to test against `content` rules (only for `send`). |
| `--force` | bool | false | Required by `init` to overwrite an existing file. |
| `--local` | bool | false | `init` writes to the project-local scope instead of global. |
| `--json` | bool | false | Emit machine-readable JSON. |

## Environment variables

| Variable | Description |
|---|---|
| `ZAD_HOME_OVERRIDE` | Override `~/` when resolving `~/.zad/`. Tests only. |
| `ZAD_SECRETS_MEMORY` | When `1`, store secrets in a process-local map instead of the OS keychain. Tests only. |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | Generic error — Bot API failure, keyring read failure, filesystem error. |
| 2 | Usage error — missing subcommand, conflicting flags, invalid chat_id, unknown name. |

## Examples

```sh
# Map a friendly name to a supergroup id.
zad telegram directory set team-room -1001234567890

# Post a message — by alias, not raw chat_id.
zad telegram send --chat team-room "deploy finished"

# Or by @username (channels / public supergroups).
zad telegram send --chat @team_notifications "deploy finished"

# Send a multi-line body via stdin (handy for CI logs).
tail -n 20 deploy.log | zad telegram send --chat team-room --stdin

# Fetch recent updates the bot has buffered (forward-only).
zad telegram read --chat team-room --limit 50 --json | jq '.messages[].body'

# List every chat zad knows about (directory cache + observed updates).
zad telegram chats --json

# Refresh the directory from the bot's current update batch.
zad telegram discover

# Scaffold a project-local permissions policy.
zad telegram permissions init --local

# Dry-run a send against the policy (exits 1 if denied).
zad telegram permissions check --function send --chat team-room --body "hi"

# Preview what would be sent without contacting the Bot API.
# `--dry-run` enforces scope + permissions, skips the keychain read,
# and prints the outgoing payload as JSON.
zad telegram send --chat team-room --dry-run "dry-run preview"
```

## See also

- `zad service` — lifecycle (`create`, `enable`, `disable`, `show`,
  `delete`, `list`).
- `zad man service` — lifecycle reference.
- `docs/configuration.md` — scopes, precedence, permissions grammar.
