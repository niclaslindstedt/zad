# zad telegram

> Runtime verbs for the Telegram service — send, read, list chats,
> join/leave, discover and curate a name → chat_id directory.

## Status

The lifecycle commands (`zad service create|enable|disable|list|show|delete telegram`)
are fully implemented. The runtime verbs below have their CLI shape
wired up but **their bodies are not implemented yet** — every verb
currently fails with `operation not supported by this service`. This is
intentional: the surface is locked in so agents can learn it, while the
Telegram Bot API client lands in a follow-up change.

Follow-ups tracked as TODOs in:

- `src/cli/telegram.rs` — per-verb plan
- `src/service/telegram/mod.rs` — client / transport / gateway / permissions plan

## Synopsis

```
zad telegram <VERB> [OPTIONS]
```

## Description

`zad telegram` operates a Telegram bot at runtime. The project must
already have Telegram enabled (`zad service enable telegram`) and valid
credentials registered in either scope. Runtime commands resolve the
effective configuration with local winning over global, then load the
matching bot token from the OS keychain.

| Verb | Description |
|---|---|
| `send` | Send a message to a chat (user, group, supergroup, or channel). |
| `read` | Fetch recent messages from a chat (via `getUpdates`). |
| `chats` | List chats the bot currently knows about (from the directory + `getChat`). |
| `join` | Join a public group or channel by `@username`. |
| `leave` | Leave a chat the bot is a member of. |
| `discover` | Walk the bot's recent updates, cache a name → chat_id map. |
| `directory` | Inspect or hand-edit that cache. |
| `permissions` | Inspect, scaffold, or dry-run the per-project permissions policy. |

Every verb supports `--json` to emit machine-readable output.

## Scope enforcement

Every runtime verb checks the required scope against the `scopes` array
in the effective credentials file **before** any network call. Missing
the scope returns a `scope denied` error that names the exact file path
to edit. The planned mapping is:

| Verb | Required scope |
|---|---|
| `send` | `messages.send` |
| `read`, `discover` | `messages.read` |
| `chats`, `join`, `leave` | `chats` |
| `manage` (library-level, not yet exposed) | `chats.manage` |
| `directory` | none (local state only) |

## Name resolution

`--chat` accepts three forms:

- A numeric Telegram chat ID. Positive for private chats with users,
  negative for groups/supergroups/channels.
- An `@username` handle, for public users, channels, or groups.
- A name from this project's directory
  (`~/.zad/projects/<slug>/services/telegram/directory.toml`), populated
  by `zad telegram discover` or by hand via `zad telegram directory set`.

## `zad telegram send`

```
zad telegram send --chat <ID|@HANDLE|NAME> [--stdin] [BODY]
```

Post a message. The body is taken from the positional argument, or from
standard input when `--stdin` is set.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--chat <id\|@handle\|name>` | chat id \| handle \| directory name | — | Destination chat. |
| `--stdin` | bool | `false` | Read the body from standard input. |
| `--json` | bool | `false` | Emit machine-readable JSON. |
| `--dry-run` | bool | `false` | Preview the outgoing call without contacting Telegram. |

## `zad telegram read`

```
zad telegram read --chat <ID|@HANDLE|NAME> [--limit N]
```

Fetch up to `--limit` recent messages from `--chat`.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--chat <id\|@handle\|name>` | chat id \| handle \| directory name | — | Chat to read from. |
| `--limit <n>` | integer | `20` | Maximum number of messages to fetch. |
| `--json` | bool | `false` | Emit machine-readable JSON. |

## `zad telegram chats`

```
zad telegram chats [--json]
```

List chats the bot currently knows about. The snapshot is built from
the project directory plus a `getChat` per known chat_id. There is no
"list all chats" endpoint in the Bot API, so this view is only as
complete as `discover` has made the directory.

## `zad telegram join` / `zad telegram leave`

```
zad telegram join  --chat <@HANDLE>
zad telegram leave --chat <ID|@HANDLE|NAME>
```

`join` requires an `@username` handle for a public chat — the Bot API
does not let bots accept private invite links. `leave` works against
any chat the bot is a member of.

## `zad telegram discover`

```
zad telegram discover [--skip-users] [--json]
```

Best-effort walk of `getUpdates` to harvest chats and users the bot has
seen. Safe to re-run; preserves hand-authored directory entries.

## `zad telegram directory`

```
zad telegram directory                                 # list
zad telegram directory set    <kind> <name> <id>       # upsert a mapping
zad telegram directory remove <kind> <name>            # delete a mapping
zad telegram directory clear  --force                  # wipe the file
```

`<kind>` is one of `chat` or `user`. Unlike Discord snowflakes,
Telegram chat IDs may be negative integers (groups and channels), so
the parser accepts a leading `-`.

## `zad telegram permissions`

Same shape as `zad discord permissions`. Function names are `send`,
`read`, `chats`, `join`, `leave`, `discover`, and `manage`. Starter
template will deny public-channel sends and all `manage`-level
operations by default.

## Environment variables

| Variable | Description |
|---|---|
| `ZAD_HOME_OVERRIDE` | Override `~/` when resolving `~/.zad/`. Tests only. |
| `ZAD_SECRETS_MEMORY` | When `1`, store secrets in a process-local map instead of the OS keychain. Tests only. |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | Generic error — Telegram API failure, keyring read failure, filesystem error, or (today) `operation not supported` for the still-TODO verbs. |
| 2 | Usage error — missing subcommand, conflicting flags, invalid chat ID, unknown name. |

## See also

- [`zad man service`](service.md) — credential management for Telegram.
- [`zad man discord`](discord.md) — sibling service with the same verb shape.
- [`zad man main`](main.md) — top-level CLI overview.
