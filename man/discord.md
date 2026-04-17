# zad discord

> Runtime verbs for the Discord service — send, read, list channels,
> join/leave threads, discover and curate a name → snowflake directory.

## Synopsis

```
zad discord <VERB> [OPTIONS]
```

## Description

`zad discord` operates the Discord service at runtime. The project must
already have Discord enabled (`zad service enable discord`) and valid
credentials registered in either scope — runtime commands resolve the
effective configuration with local winning over global, then load the
matching bot token from the OS keychain.

| Verb | Description |
|---|---|
| `send` | Send a message to a channel or a direct message to a user. |
| `read` | Fetch recent messages from a channel. |
| `channels` | List every channel in a guild (text, voice, threads, categories). |
| `join` | Join a thread channel. |
| `leave` | Leave a thread channel. |
| `discover` | Walk the bot's visible guilds/channels/members and cache a name → snowflake map. |
| `directory` | Inspect or hand-edit that cache. |

Every verb supports `--json` to emit machine-readable output instead
of the human-readable default.

## Name resolution

`--channel`, `--dm`, and `--guild` all accept either a numeric snowflake or
a name from this project's directory
(`~/.zad/projects/<slug>/services/discord/directory.toml`). Channel names
may be bare (`general`) or guild-qualified (`main-server/general`); user
names may be prefixed with `@` and channel names with `#` for ergonomic
pasting (`#general`, `@alice`). When the name is unknown, the error
message prints the exact `zad discord directory set …` command that would
map it.

## `zad discord send`

```
zad discord send (--channel <ID|NAME> | --dm <USER|NAME>) [--stdin] [BODY]
```

Post a message. Exactly one of `--channel` or `--dm` is required. The
body is taken from the positional argument, or from standard input when
`--stdin` is set.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--channel <id\|name>` | snowflake \| directory name | — | Destination channel. Mutually exclusive with `--dm`. |
| `--dm <id\|name>` | snowflake \| directory name | — | Destination user for a DM. Mutually exclusive with `--channel`. |
| `--stdin` | bool | `false` | Read the body from standard input instead of a positional argument. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

## `zad discord read`

```
zad discord read --channel <ID|NAME> [--limit N]
```

Fetch up to `--limit` recent messages from `--channel` (Discord caps
this at 100). Output is printed in chronological order (oldest first)
so a terminal reader sees the natural flow.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--channel <id\|name>` | snowflake \| directory name | — | Channel to read from. |
| `--limit <n>` | integer | `20` | Maximum number of messages to fetch (1–100). |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

## `zad discord channels`

```
zad discord channels [--guild <ID|NAME>]
```

List every channel visible to the bot in `--guild`. Falls back to the
service config's `default_guild` when no flag is passed. Output columns
are `ID`, `KIND` (one of `text`, `voice`, `category`, `news`,
`public_thread`, `private_thread`, `news_thread`, `stage`, `forum`,
`directory`, `unknown`), and `NAME`.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--guild <id\|name>` | snowflake \| directory name | `default_guild` from the effective config | Guild (server) whose channels to list. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of the human-readable table. |

## `zad discord join` / `zad discord leave`

```
zad discord join --channel <ID|NAME>
zad discord leave --channel <ID|NAME>
```

Join or leave a **thread** channel. Discord only supports explicit
join/leave on thread members; regular guild text and voice channels
are joined implicitly by having the guild membership and the right
permissions, so the commands error for non-thread channel IDs.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--channel <id\|name>` | snowflake \| directory name | — | Thread channel to join or leave. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

## `zad discord discover`

```
zad discord discover [--guild <ID|NAME>] [--skip-members] [--json]
```

Walk the bot's visible guilds, then for each guild list its channels and
(when the bot has the `GUILD_MEMBERS` privileged intent enabled) its
members. Every name → snowflake it learns is written into this project's
`directory.toml`, merged on top of any hand-authored entries already
there. Failures on a single endpoint are logged as warnings on stderr and
the walk continues — this is a **best-effort** surface, safe to re-run.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--guild <id\|name>` | snowflake \| directory name | — | Scope the channel/member walk to a single guild. Every guild is still added to `[guilds]` so the name is resolvable. |
| `--skip-members` | bool | `false` | Skip the member-listing phase (suppresses the "needs `GUILD_MEMBERS` intent" warning when the bot doesn't have it). |
| `--json` | bool | `false` | Emit a JSON summary (counts plus per-endpoint warnings). |

Output (human):

```
Wrote directory: 2 guilds, 42 channel entries, 128 users.
warning: members for `staging` (needs GUILD_MEMBERS privileged intent): 403 Forbidden
```

## `zad discord directory`

```
zad discord directory                                 # list
zad discord directory set    <kind> <name> <id>       # upsert a mapping
zad discord directory remove <kind> <name>            # delete a mapping
zad discord directory clear  --force                  # wipe the file
```

`<kind>` is one of `guild`, `channel`, `user`. Channel names may be bare
(`general`) or guild-qualified (`main-server/general`); the qualified
form wins at lookup time when the caller has a guild context.

`remove` is idempotent — removing an entry that was never there is not an
error, so agent scripts don't have to pre-check.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--force` | bool | `false` | Required by `clear` to confirm wiping the directory. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

## Environment variables

| Variable | Description |
|---|---|
| `ZAD_HOME_OVERRIDE` | Override `~/` when resolving `~/.zad/`. Tests only. |
| `ZAD_SECRETS_MEMORY` | When `1`, store secrets in a process-local map instead of the OS keychain. Tests only. |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | Generic error — Discord API failure, keyring read failure, filesystem error. |
| 2 | Usage error — missing subcommand, conflicting flags, invalid numeric ID, unknown name. |

## Examples

```sh
# Populate the name -> snowflake directory (best-effort, re-runnable)
zad discord discover

# Manually add an entry the bot can't see (e.g. a user it's never DM'd)
zad discord directory set user alice 1234567890

# Post a message to a channel — by name, not snowflake
zad discord send --channel general "deploy finished"

# Or by snowflake, same flag
zad discord send --channel 1111111111111111 "deploy finished"

# Send a multi-line body via stdin (handy for CI logs)
tail -n 20 deploy.log | zad discord send --channel general --stdin

# DM a user directly
zad discord send --dm @alice "standup in 5 minutes"

# Read recent history from a channel
zad discord read --channel general --limit 50 --json | jq '.messages[].body'

# List channels in a guild (falls back to default_guild from the config)
zad discord channels --json

# Join and leave a thread channel
zad discord join --channel 3333333333333333
zad discord leave --channel 3333333333333333
```

## See also

- [`zad man service`](service.md) — credential management for Discord.
- [`zad man main`](main.md) — top-level CLI overview.
- [`docs/configuration.md`](../docs/configuration.md) — config file reference (includes the `directory.toml` schema).
