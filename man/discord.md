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
| `permissions` | Inspect, scaffold, or dry-run the per-project permissions policy. |
| `self` | Manage the Discord user ID resolved from the literal `@me` in `--dm` targets. |

Every verb supports `--json` to emit machine-readable output instead
of the human-readable default.

## Scope enforcement

Every runtime verb checks the required scope against the `scopes` array
in the effective credentials file **before** any network call. Missing
the scope returns a `scope denied` error that names the exact file path
to edit. The mapping is:

| Verb | Required scope |
|---|---|
| `send` | `messages.send` |
| `read` | `messages.read` |
| `channels`, `discover`, `join`, `leave` | `guilds` |
| `directory` | none (local state only) |

See `docs/configuration.md` for the full scope list and for the
local-vs-global precedence rules.

## Permissions (second layer)

Scope is the coarse gate — "is this family of operations enabled?".
**Permissions** are the fine gate — "is *this* call (to this target, at
this time, with this content) allowed?". They live in an optional
TOML file at:

- Global: `~/.zad/services/discord/permissions.toml`
- Local:  `~/.zad/projects/<slug>/services/discord/permissions.toml`

Both files apply — a call must pass every file that exists. Missing
files contribute no restrictions. The `docs/configuration.md` file
documents the full schema (allow/deny globs and regex, denied content
words and patterns, UTC time windows, per-function blocks). The mapping
from verb to function block is:

| Verb | Permissions block | Matches against |
|---|---|---|
| `send`     | `[send]`     | `channels` (for `--channel`) or `users` (for `--dm`); body against `content`; files against `[send.attachments]` |
| `read`     | `[read]`     | `channels` |
| `channels` | `[channels]` | `guilds` |
| `join`     | `[join]`     | `channels` |
| `leave`    | `[leave]`    | `channels` |
| `discover` | `[discover]` | `guilds` — denied guilds are silently skipped in the walk |
| (library-level `manage`) | `[manage]` | `channels` |

Permission violations surface with a `permission denied` error that
names the function, the reason, and the exact file path to edit — the
same shape as the scope-denied error.

## Name resolution

`--channel`, `--dm`, and `--guild` all accept either a numeric snowflake or
a name from this project's directory
(`~/.zad/projects/<slug>/services/discord/directory.toml`). Channel names
may be bare (`general`) or guild-qualified (`main-server/general`); user
names may be prefixed with `@` and channel names with `#` for ergonomic
pasting (`#general`, `@alice`). When the name is unknown, the error
message prints the exact `zad discord directory set …` command that would
map it.

`--dm` also accepts the literal `@me` (case-insensitive), which resolves
to the snowflake stored via `zad discord self set` (or `--self-user` on
`service create`). Errors with a pointer to `self set` when no
self-user is configured.

## `zad discord send`

```
zad discord send (--channel <ID|NAME> | --dm <USER|NAME>) [--stdin] [--file PATH]... [BODY]
```

Post a message. Exactly one of `--channel` or `--dm` is required. The
body is taken from the positional argument, or from standard input when
`--stdin` is set. Bodies longer than Discord's 2000-codepoint hard limit
are rejected locally (no round-trip).

Pass `--file PATH` one or more times to attach files. Discord accepts
at most 10 attachments per message; zad rejects anything above that
before touching the network. When at least one `--file` is given the
message body may be empty.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--channel <id\|name>` | snowflake \| directory name | — | Destination channel. Mutually exclusive with `--dm`. |
| `--dm <id\|name>` | snowflake \| directory name | — | Destination user for a DM. Mutually exclusive with `--channel`. |
| `--stdin` | bool | `false` | Read the body from standard input instead of a positional argument. |
| `--file <path>` | path | — | Attach a file. Repeat up to 10 times. When present the body is optional. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |
| `--dry-run` | bool | `false` | Preview the outgoing call without contacting Discord — prints the payload as JSON on stdout (including an `attachments` array with `path`, `basename`, and `bytes` per file) and makes no network request. Scope and permission checks still run; no bot token is loaded (so the flag works before a bot is configured). The trailing `Sent message …` line is suppressed. |

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
| `--limit <n>` | integer | `20` | Maximum number of messages to fetch (1–100). Values outside that range are rejected locally with exit code 2. |
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
| `--dry-run` | bool | `false` | Preview the outgoing call without contacting Discord. Scope and permission checks still run; no bot token is loaded. |

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

## `zad discord permissions`

```
zad discord permissions                          # show (same as `show`)
zad discord permissions show
zad discord permissions path
zad discord permissions init  [--local] [--force]
zad discord permissions check --function <name> [--channel|--user|--guild <id|name>] [--body <text>]

# Staged-commit workflow (shared across every service — see docs/permissions.md)
zad discord permissions status   [--local]
zad discord permissions diff     [--local]
zad discord permissions discard  [--local]
zad discord permissions commit   [--local]       # signs + atomic replace
zad discord permissions sign     [--local]       # re-sign after hand edit
zad discord permissions add      --function <f> --target <channel|user|guild> --list <allow|deny> [--local] <pattern>
zad discord permissions remove   --function <f> --target <channel|user|guild> --list <allow|deny> [--local] <pattern>
zad discord permissions content  [--function <f>] [--local] {add-deny-word WORD|remove-deny-word WORD|add-deny-regex PAT|remove-deny-regex PAT|set-max-length --value N|set-max-length --clear}
zad discord permissions time     [--function <f>] [--local] {set-days --days mon,tue,... | set-windows --windows 09:00-18:00,...}
```

- `show` — print both candidate file paths plus the body of whichever
  files exist.
- `path` — print only the two candidate paths, one per line.
- `init` — write a starter policy. Defaults to the global scope; pass
  `--local` to target `~/.zad/projects/<slug>/services/discord/`. The
  template denies admin-like channels and all `channels.manage`
  operations. On first run `init` also generates a machine-wide
  Ed25519 signing keypair in your OS keychain (account `signing:v1`)
  and signs the starter template. Subsequent `init` calls reuse the
  same keypair. See [`docs/permissions.md`](../docs/permissions.md)
  for the trust model.
- `check` — dry-run a proposed action against the effective policy.
  Exits 0 on allow, 1 on deny with the reason and the config path
  printed.

| Flag | Type | Default | Description |
|---|---|---|---|
| `--function <name>` | string | — | One of `send`, `read`, `channels`, `join`, `leave`, `discover`, `manage`. |
| `--channel <id\|name>` | snowflake \| directory name | — | Target channel for channel-scoped functions. |
| `--user <id\|name>` | snowflake \| directory name | — | Target user for `send --dm`. Mutually exclusive with `--channel`. |
| `--guild <id\|name>` | snowflake \| directory name | — | Target guild for `channels` / `discover`. |
| `--body <text>` | string | — | Message body to test against `content` rules (only for `send`). |
| `--force` | bool | false | Required by `init` to overwrite an existing file. |
| `--local` | bool | false | `init` writes to the project-local scope instead of global. |
| `--json` | bool | false | Emit machine-readable JSON. |

## `zad discord self`

```
zad discord self                       # show (same as `show`)
zad discord self show
zad discord self set     <USER_ID>     # validates against Discord before storing
zad discord self clear
```

Manage the Discord user ID that `--dm @me` resolves to. Stored as
`self_user_id` in the effective `config.toml` (non-secret).

- `show` — print the stored value or `"not configured"`.
- `set <user_id>` — validate the snowflake against `GET /users/{id}`
  and, on success, persist it. Fails cleanly on an unknown or
  non-numeric ID. Requires a stored bot token.
- `clear` — remove the stored value.

Find your own user ID in Discord: Settings → Advanced → enable
Developer Mode, then right-click yourself → "Copy User ID".

| Flag | Type | Default | Description |
|---|---|---|---|
| `--json` | bool | false | Emit machine-readable JSON on every subcommand. |

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

# Attach one or more files (up to 10). Body is optional when a file is attached.
zad discord send --channel general --file ./report.pdf "see attached"
zad discord send --channel general --file ./a.log --file ./b.png

# DM a user directly
zad discord send --dm @alice "standup in 5 minutes"

# DM yourself (after `zad discord self set <your-user-id>`)
zad discord send --dm @me "remember to file the time sheet"

# Scaffold a local permissions policy, then dry-run a send
zad discord permissions init --local
zad discord permissions check --function send --channel general --body "hello"

# Read recent history from a channel
zad discord read --channel general --limit 50 --json | jq '.messages[].body'

# List channels in a guild (falls back to default_guild from the config)
zad discord channels --json

# Join and leave a thread channel
zad discord join --channel 3333333333333333
zad discord leave --channel 3333333333333333

# Preview what would be sent without actually contacting Discord.
# `--dry-run` enforces scope + permissions, skips the keychain read,
# and prints the outgoing payload as JSON. Works for send / join / leave.
zad discord send --channel general --dry-run "dry-run preview"
zad discord join  --channel 3333333333333333 --dry-run
```

## See also

- [`zad man service`](service.md) — credential management for Discord.
- [`zad man main`](main.md) — top-level CLI overview.
- [`docs/configuration.md`](../docs/configuration.md) — config file reference (includes the `directory.toml` schema).
