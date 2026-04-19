# Configuration

zad stores per-project service configuration in a TOML file under the
user's home directory:

```
~/.zad/projects/<slug>/config.toml
```

`<slug>` is the absolute current working directory with every `/` (and
every `\` or `:` on Windows) replaced by `-` — the same convention Claude
Code uses for its per-project files. For example, working in
`/Users/alice/code/zad` yields the slug `-Users-alice-code-zad`.

Secrets (bot tokens, API keys) are **never** written to the TOML. They
live in the OS keychain under the `zad` service.

## Resolution

| Override | Effect |
|---|---|
| `ZAD_HOME_OVERRIDE` | Replaces `~/` when computing `~/.zad/`. Tests only. |
| `ZAD_SECRETS_MEMORY=1` | Swaps the OS keyring for a process-local in-memory store. Tests only. |

## Discord service

Commands that drive it (documented in [`man/service.md`](../man/service.md) and [`man/discord.md`](../man/discord.md)):

- `zad service create discord [--local]` — register credentials.
- `zad service enable discord` — enable the service in the current project.
- `zad service disable discord` — disable it again (leaves credentials intact).

Every command accepts `--json` for script-friendly structured output.

### Credentials file

Stored at **one** of:

- Global: `~/.zad/services/discord/config.toml`
- Local:  `~/.zad/projects/<slug>/services/discord/config.toml`

The project-local file wins over the global one for that project. The
format is flat (no `[service.discord]` wrapper — the path already
identifies the service):

```toml
application_id = "1234567890"
scopes         = ["guilds", "messages.read", "messages.send"]
default_guild  = "987654321"          # optional
self_user_id   = "1112223334445556"   # optional — resolved from `@me` in --dm targets
```

| Key | Type | Default | Description |
|---|---|---|---|
| `application_id` | string | — | Discord application (bot) ID. Numeric snowflake. |
| `scopes` | `[string]` | `["guilds", "messages.read", "messages.send"]` | Capabilities the service is permitted to use. |
| `default_guild` | string? | — | Optional default guild (server) ID. |
| `self_user_id` | string? | — | Your own Discord user ID. Resolved from the literal `@me` in `send --dm`. Populated at `service create` time (flag or prompt) or later via `zad discord self set <id>`. Validated against `GET /users/{id}` before being written. |

Scopes are **enforced at runtime, before any network call**. Omitting a
scope denies the corresponding operation locally with a `scope denied`
error that names the config path — Discord's OAuth permissions are not
trusted on their own. The supported values are:

| Scope | Gates |
|---|---|
| `guilds` | `channels`, `join`, `leave`, `discover` (listing guilds, channels, members) |
| `messages.read` | `read` (channel history) |
| `messages.send` | `send` (channel or DM) |
| `channels.manage` | Creating or deleting channels (library-level only; no CLI verb today) |
| `gateway.listen` | Gateway event listener (library-level only) |

When both a global and a project-local credentials file exist, the local
file **replaces** the global one for that project — scopes are not
merged. Write the full scope set each time.

### Permissions file

Scopes answer "is this family of operations enabled at all?".
Permissions are a second, finer layer — *which channels, which users,
which times, which content* — that a declared scope may actually act
on. They live in an optional file next to the credentials:

- Global: `~/.zad/services/discord/permissions.toml`
- Local:  `~/.zad/projects/<slug>/services/discord/permissions.toml`

Unlike credentials, **both files apply**: a call must pass every file
that exists. This makes it safe to ship a strict global baseline — a
project can only add further restrictions, never loosen the global
rule. An absent file contributes no restrictions; when both are absent,
scope is the only gate.

A complete worked example lives at
[`examples/discord-permissions/`](../examples/discord-permissions/).

The schema is a small TOML file with top-level defaults plus one block
per function:

```toml
# Shared defaults. Each per-function block inherits from these and can
# add further narrowing.
[content]
deny_words    = ["password", "api_key", "secret"]
deny_patterns = ["(?i)bearer\\s+[a-z0-9]+"]
max_length    = 1500      # codepoints; narrows Discord's 2000 hard cap

[time]
days    = ["mon", "tue", "wed", "thu", "fri"]
windows = ["09:00-18:00"]  # UTC

# Per-function blocks. Each has channels / users / guilds sublists
# (whichever apply to the function) plus optional content / time
# overrides that **narrow** the top-level defaults.
[send]
channels.allow = ["general", "bot-*", "team/*"]
channels.deny  = ["*admin*", "mod-*"]
users.allow    = ["alice", "bob"]

[read]
channels.deny = ["*private*"]

[channels]
guilds.allow = ["main-server"]

[join]
channels.deny = ["*admin*"]

[leave]
# no restrictions

[discover]
guilds.allow = ["main-server"]

[manage]
# Default-deny for channels.manage: nothing is touched unless allowed.
channels.allow = []
channels.deny  = ["*"]
```

Pattern grammar (used anywhere an allow/deny list appears):

| Form | Meaning |
|---|---|
| `general` | Exact name match. |
| `bot-*`, `team/*` | Glob: `*` and `?` wildcards. Other regex metacharacters are escaped. |
| `re:<regex>` | Full Rust regex syntax. Anchor it yourself if you need to (`re:^mod-[0-9]+$`). |
| `1234567890` | Numeric — matches the resolved snowflake exactly. |

Evaluation order:

1. If any **deny** pattern matches, the call is denied. Deny always wins.
2. If the **allow** list is empty, there is no positive constraint —
   the call passes on this front.
3. Otherwise the call must match at least one allow pattern.

A rule is evaluated against every alias of the target: the input the
agent typed (with `#` or `@` sigils stripped), the resolved snowflake
as a string, and every name the `directory.toml` has for that
snowflake. So a deny on `*admin*` fires even when the agent passes the
raw snowflake, as long as the directory knows the ID under an
admin-like name.

Content rules (`deny_words`, `deny_patterns`, `max_length`) apply to
outbound message bodies. `deny_words` is case-insensitive substring
matching; `deny_patterns` is full regex; `max_length` is measured in
codepoints and only *tightens* Discord's 2000-char ceiling.

The `[time]` block pins a UTC business-hours window. An empty `days`
list admits every day; an empty `windows` list admits the whole day.
Windows may cross midnight (`22:00-02:00`).

Manage permissions from the CLI:

- `zad discord permissions show` — print the effective policy (both
  scopes).
- `zad discord permissions path` — print the two candidate paths.
- `zad discord permissions init [--local] [--force]` — write a
  starter policy. The default template denies admin-like channels and
  all `channels.manage` operations.
- `zad discord permissions check --function <name> [--channel|--user|--guild <id|name>] [--body TEXT]` —
  dry-run: returns allow/deny and the config path that decided, without
  hitting Discord. Intended for agents that want to pre-flight an
  action.

When a runtime verb is denied, the error message names the function,
the deny reason, and the exact file path to edit — the same shape as
the scope-denied error.

### Project file

`~/.zad/projects/<slug>/config.toml` records which services are enabled
for the project. It never contains credentials.

```toml
[service.discord]
enabled = true
```

### Token storage

The bot token is stored in the OS keychain at:

- **service:** `zad`
- **account:** `discord-bot:global` (global creds) or `discord-bot:<slug>` (local creds).

Rotate a token by re-running `zad service create discord --force` (add
`--local` to target project-local credentials).

### Directory (name -> snowflake)

`zad discord discover` walks the bot's visible guilds/channels/members
and writes a local directory file at:

```
~/.zad/projects/<slug>/services/discord/directory.toml
```

The file is plain TOML and is the canonical source for ergonomic names
used by `--channel`, `--dm`, and `--guild` on every runtime verb. It is
safe to hand-edit; `discover` upserts on top of existing entries rather
than overwriting the file.

```toml
generated_at_unix = 1713364920   # optional; set by `discover`

[guilds]
"main-server" = "999000000000000000"

[channels]
# "guild/channel" wins over "channel" when both exist and a guild
# context is known. A bare `general` still resolves when the caller
# doesn't pass a guild.
"main-server/general"   = "111000000000000000"
"main-server/announce"  = "112000000000000000"
"general"               = "111000000000000000"

[users]
"alice" = "1001000000000000000"
```

Manage it from the CLI:

- `zad discord directory` — list every entry.
- `zad discord directory set <kind> <name> <id>` — upsert, where
  `<kind>` is `guild`, `channel`, or `user`.
- `zad discord directory remove <kind> <name>` — idempotent delete.
- `zad discord directory clear --force` — wipe the file.

Member discovery uses the Discord `GET /guilds/{id}/members` endpoint,
which requires the **GUILD_MEMBERS** privileged intent to be enabled for
the bot in the developer portal. Without it, `discover` skips the
members phase and emits a one-line warning — it is explicitly
best-effort and never aborts the walk.

### Privileged intents

Reading message *content* from guild channels requires the
**MESSAGE_CONTENT** privileged intent to be enabled for the bot in the
Discord developer portal. Without it, the `body` field on gateway
`MessageCreated` events is empty for guild messages.

## Telegram service

Commands that drive it (documented in [`man/service.md`](../man/service.md) and [`man/telegram.md`](../man/telegram.md)):

- `zad service create telegram [--local]` — register credentials.
- `zad service enable telegram` — enable the service in the current project.
- `zad service disable telegram` — disable it again (leaves credentials intact).

Telegram bots carry their identity inside the bot token itself, so
the credentials file is shorter than Discord's — no `application_id`.

### Credentials file

Stored at **one** of:

- Global: `~/.zad/services/telegram/config.toml`
- Local:  `~/.zad/projects/<slug>/services/telegram/config.toml`

The project-local file wins over the global one for that project. The
format is flat:

```toml
scopes       = ["chats", "messages.read", "messages.send"]
default_chat = "team-room"    # optional
self_chat_id = 8675309        # optional — resolved from `@me` in --chat targets
```

| Key | Type | Default | Description |
|---|---|---|---|
| `scopes` | `[string]` | `["chats", "messages.read", "messages.send"]` | Capabilities the service is permitted to use. |
| `default_chat` | string? | — | Optional default destination for `send`. Accepts a signed chat_id (negative for groups/supergroups), a public `@username`, or a directory alias. |
| `self_chat_id` | i64? | — | Your own private-chat ID with this bot. Resolved from the literal `@me` in `send`/`read` targets. Captured interactively at `service create` time via a `getUpdates` poll (or set directly with `--self-chat`), and can be managed later via `zad telegram self capture|set|clear`. |

Scopes are **enforced at runtime, before any network call**. The
supported values are:

| Scope | Gates |
|---|---|
| `messages.send` | `send` |
| `messages.read` | `read` |
| `chats` | `chats`, `discover` (and any future chat-listing verb) |
| `gateway.listen` | Gateway event listener (library-level only; no CLI verb today) |

When both a global and a project-local credentials file exist, the
local file **replaces** the global one for that project — scopes are
not merged.

### Permissions file

The permissions layer has the same shape as Discord's (see above),
with one per-verb block per runtime verb:

| Block | Narrows |
|---|---|
| `[send]`     | `chats` allow/deny for the destination; body against `content` |
| `[read]`     | `chats` allow/deny for the source |
| `[chats]`    | `chats` allow/deny for the listing |
| `[discover]` | `chats` allow/deny — denied chats are silently skipped in the walk |

See [`examples/telegram-permissions/`](../examples/telegram-permissions/)
for a worked example.

### Project file

The same `~/.zad/projects/<slug>/config.toml` that records Discord
enablement records Telegram the same way:

```toml
[service.telegram]
enabled = true
```

### Token storage

The bot token is stored in the OS keychain at:

- **service:** `zad`
- **account:** `telegram-bot:global` (global creds) or `telegram-bot:<slug>` (local creds).

Rotate a token by re-running `zad service create telegram --force`
(add `--local` to target project-local credentials).

### Directory (name -> chat_id)

`zad telegram discover` polls the Bot API for recent updates and
upserts a local directory file at:

```
~/.zad/projects/<slug>/services/telegram/directory.toml
```

Telegram addresses every target through a single signed `chat_id`
(negative for groups and supergroups, positive for private chats and
most channels), so the file has one `chats` map rather than splitting
by target kind.

```toml
generated_at_unix = 1713364920   # optional; set by `discover`

[chats]
"team-room"            = "-1001234567890"
"announcements"        = "-1009876543210"
"alice"                = "1001"
```

Manage it from the CLI:

- `zad telegram directory` — list every entry.
- `zad telegram directory set <name> <id>` — upsert a mapping.
- `zad telegram directory remove <name>` — idempotent delete.
- `zad telegram directory clear --force` — wipe the file.

### Bot API caveats

Telegram's Bot API exposes `getUpdates` as a forward-only stream —
there is no "give me the last N messages" endpoint. `zad telegram
read` therefore returns only what the bot has buffered since its
previous `getUpdates` call, and `zad telegram chats` / `discover`
likewise see only chats present in the current update batch. The
manpage documents the "new messages only" shape explicitly.

## Logging

zad always writes a rolling daily log file at a platform-appropriate
state directory (per `OSS_SPEC.md` §19.2):

| Platform | Path |
|---|---|
| Linux   | `~/.local/state/zad/debug.log` |
| macOS   | `~/Library/Application Support/zad/debug.log` |
| Windows | `%LOCALAPPDATA%\zad\debug.log` |

The global `--debug` flag additionally mirrors the log to stderr.
