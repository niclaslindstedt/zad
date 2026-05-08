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
| `ZAD_PERMISSIONS_PATH` | Pin the local permissions file to the given path. Bypasses the cwd-derived project slug so the same policy applies regardless of which directory `zad` runs from. Schema must match the service being invoked. Wins over `ZAD_PERMISSIONS_ROOT`. |
| `ZAD_PERMISSIONS_ROOT` | Pin the local permissions root; resolved as `<root>/<service>/permissions.toml`. Lets one env var cover every service. |

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

# Per-function attachment policy. Each field is optional; an absent
# block means "no attachment-specific restrictions". Discord hard-caps
# attachments at 10 per message regardless of this setting.
[send.attachments]
max_count      = 5
max_size_bytes = 8388608                                    # 8 MiB per file
extensions     = { allow = ["png", "jpg", "txt", "log", "md", "json"], deny = ["exe", "dll", "sh"] }
deny_filenames = { deny = [".env*", "id_rsa*", "*.pem"] }

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
| `[send]`     | `chats` allow/deny for the destination; body against `content`; files against `[send.attachments]` |
| `[read]`     | `chats` allow/deny for the source |
| `[chats]`    | `chats` allow/deny for the listing |
| `[discover]` | `chats` allow/deny — denied chats are silently skipped in the walk |

The `[send.attachments]` sub-block has the same shape and semantics as
the Discord version documented above: optional `max_count`,
`max_size_bytes`, `extensions = { allow, deny }`, and
`deny_filenames = { deny }`. Telegram caps `sendMediaGroup` at 10 files
regardless of this setting; single-file sends are routed through
`sendDocument` (body becomes the caption, capped at 1024 characters).

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

## Slack service

Commands that drive it (documented in [`man/service.md`](../man/service.md) and [`man/slack.md`](../man/slack.md)):

- `zad service create slack [--local]` — register credentials.
- `zad service enable slack` — enable the service in the current project.
- `zad service disable slack` — disable it again (leaves credentials intact).

### Credentials file

Stored at **one** of:

- Global: `~/.zad/services/slack/config.toml`
- Local:  `~/.zad/projects/<slug>/services/slack/config.toml`

The project-local file wins over the global one for that project. The
format is flat:

```toml
app_id          = "A012345678"
workspace       = "my-team"
scopes          = ["chat:write", "channels:history", "channels:read"]
default_channel = "C1234567890"   # optional
self_user_id    = "U9876543210"   # optional — resolved from `@me` in --dm targets
```

| Key | Type | Default | Description |
|---|---|---|---|
| `app_id` | string | required | Slack app ID (starts with `A`). Used to construct the install URL. |
| `workspace` | string | required | Team/workspace name or domain (display only). |
| `scopes` | `[string]` | `["chat:write", "channels:history", "channels:read"]` | Capabilities the service is permitted to use. |
| `default_channel` | string? | — | Optional default destination for `send`. Accepts a `C...` channel ID or a directory alias. |
| `self_user_id` | string? | — | Your own Slack user ID (`U...`). Resolved from the literal `@me` in `--dm` targets. Set via `zad slack self set <id>` or during `service create`. |

Scopes are **enforced at runtime, before any network call**. The
supported values are:

| Scope | Gates |
|---|---|
| `chat:write` | `send` (channels and DMs) |
| `channels:history` | `read` (public channels) |
| `im:history` | `read` (DM channels) |
| `channels:read` | `channels`, `discover` (channel listing) |
| `im:write` | `send` DMs (`conversations.open`) |
| `users:read` | `discover` (member listing) |
| `channels:join` | Auto-join channels before posting |
| `reactions:write` | Add emoji reactions |
| `team:read` | Workspace metadata |

When both a global and a project-local credentials file exist, the
local file **replaces** the global one for that project — scopes are
not merged.

### App-Level Token (Socket Mode)

Real-time event listening via `zad slack listen` requires an App-Level
Token (`xapp-...`) in addition to the bot token. Without it,
`listen()` returns an empty stream with a warning.

To enable Socket Mode:

1. In your Slack app dashboard → Socket Mode → Enable.
2. Create an App-Level Token with `connections:write` scope.
3. Supply it via `--app-token` when running `zad service create slack`,
   or update it later with `zad service create slack --force --app-token <token>`.

The app-level token is stored separately in the keychain (see Token
storage below).

### Permissions file

The permissions layer sits on top of scopes and has the same two-scope
structure as Discord/Telegram. Both the global and local files apply
simultaneously — strictest wins. Initialize a starter file:

```sh
zad slack permissions init
```

Per-verb blocks:

| Block | Narrows |
|---|---|
| `[send]` | `channels` and `users` allow/deny for the destination; body against `[content]` |
| `[read]` | `channels` allow/deny for the source |
| `[channels]` | workspace-level listing — `workspaces` allow/deny |
| `[discover]` | workspace-level walk — `workspaces` allow/deny |

Top-level `[content]` and `[time]` defaults apply to all verbs unless
a per-verb block overrides them. Pattern kinds: exact name, glob
(`*`, `?`), and `re:<regex>`.

See [`examples/slack-permissions/`](../examples/slack-permissions/)
for a worked example.

### Project file

The same `~/.zad/projects/<slug>/config.toml` that records Discord and
Telegram enablement records Slack the same way:

```toml
[service.slack]
enabled = true
```

### Token storage

Bot and app-level tokens are stored in the OS keychain:

| Token kind | Account key (global) | Account key (local) |
|---|---|---|
| Bot token (`xoxb-...`) | `slack-bot:global` | `slack-bot:<slug>` |
| App-Level Token (`xapp-...`) | `slack-app:global` | `slack-app:<slug>` |

Rotate a token by re-running `zad service create slack --force` (add
`--local` to target project-local credentials).

### Directory (name → Slack ID)

`zad slack discover` walks the workspace's channels and users and
upserts a local directory file at:

```
~/.zad/projects/<slug>/services/slack/directory.toml
```

The file splits entries by target kind:

```toml
generated_at_unix = 1713364920   # optional; set by `discover`

[channels]
"general"  = "C1234567890"
"random"   = "C0987654321"
"bot-ops"  = "C1111111111"

[users]
"alice"    = "U2222222222"
"bob"      = "U3333333333"
```

Manage it from the CLI:

- `zad slack directory` — list every entry.
- `zad slack directory set channel <name> <id>` — upsert a channel mapping.
- `zad slack directory set user <name> <id>` — upsert a user mapping.
- `zad slack directory remove channel <name>` — idempotent delete.
- `zad slack directory remove user <name>` — idempotent delete.
- `zad slack directory clear --force` — wipe the file.

Channel resolution order for `--channel` and `--dm`: (1) raw Slack ID
(`C...`, `U...`, etc.), (2) the literal `default` to use
`default_channel`, (3) directory lookup after stripping any leading `#`
or `@`.

## Google Calendar service (`gcal`)

Commands that drive it (documented in [`man/service.md`](../man/service.md) and [`man/gcal.md`](../man/gcal.md)):

- `zad service create gcal [--local]` — register OAuth credentials
  (interactive browser flow or `--refresh-token`).
- `zad service enable gcal` — enable the service in the current project.
- `zad service disable gcal` — disable it again (leaves credentials intact).

Every command accepts `--json` for script-friendly structured output.

### Credentials file

Stored at **one** of:

- Global: `~/.zad/services/gcal/config.toml`
- Local:  `~/.zad/projects/<slug>/services/gcal/config.toml`

The project-local file wins over the global one for that project. The
format is flat:

```toml
scopes          = ["calendars.read", "events.read", "events.write"]
default_calendar = "primary"             # optional
self_email       = "alice@example.com"   # optional — resolved from `@me` in attendee targets
```

| Key | Type | Default | Description |
|---|---|---|---|
| `scopes` | `[string]` | `["calendars.read", "events.read", "events.write"]` | Capabilities the service is permitted to use. |
| `default_calendar` | string? | — | Optional default calendar ID (`primary`, an email, or an alias). Runtime verbs that omit `--calendar` use this. |
| `self_email` | string? | — | The authenticated user's email. Populated from Google's userinfo endpoint during `service create`; resolves `@me` in `--attendee` / `--add-attendee`. |

Unlike the bot-token services, `gcal` stores **three** keychain
entries per scope — the OAuth 2.0 client_id + client_secret +
refresh_token. Access tokens are not persisted: each CLI invocation
refreshes once and uses the token for that process lifetime.

| Keychain account | Contents |
|---|---|
| `gcal-client-id:<scope>` | OAuth client ID. |
| `gcal-client-secret:<scope>` | OAuth client secret. |
| `gcal-refresh:<scope>` | Refresh token. |

Scopes are **enforced at runtime, before any network call**. The
supported values are:

| Scope | Gates | Google OAuth scope |
|---|---|---|
| `calendars.read` | `calendars list`, `calendars show` | `calendar.calendarlist.readonly` (or a broader scope if `events.write` is also set) |
| `events.read` | `events list`, `events show` | `calendar.events.readonly` |
| `events.write` | `events create`, `events update`, `events delete` | `calendar.events` |
| `events.invite` | `--add-attendee` on `events update` (plus attendee additions on `events create`) | none (pure zad policy gate) |
| `events.remind` | `--reminder-minutes` / `--add-reminder-minutes` | none (pure zad policy gate) |

When both a global and a project-local credentials file exist, the
local file **replaces** the global one for that project — scopes are
not merged. Write the full scope set each time.

### Creating the Google OAuth client

`zad service create gcal` expects a Google Cloud **"Desktop app"**
OAuth client. Any other client type will fail at token exchange with
`redirect_uri_mismatch`:

1. Visit <https://console.cloud.google.com/apis/credentials>.
2. **Create credentials → OAuth client ID → Application type: Desktop app.**
3. Enable the **Google Calendar API** under "APIs & Services → Library".
4. Copy the Client ID + Client Secret. `zad service create gcal`
   prompts for both, then opens your browser to authorize, captures
   the code on `http://127.0.0.1:<port>` (PKCE S256, random 32-byte
   state), and stores the returned refresh token.

For CI or non-interactive use, mint a refresh token out-of-band
(e.g. via Google's OAuth Playground, selecting the Calendar scopes
matching your declared zad scopes) and pass
`--client-id`/`--client-secret`/`--refresh-token`.

### Permissions file

Scopes answer "is this family of operations enabled at all?".
Permissions are a second, finer layer — *which calendars, which
attendees, at what time, with what content, how far in the future,
with how much notice* — and live in an optional TOML file next to
the credentials:

- Global: `~/.zad/services/gcal/permissions.toml`
- Local:  `~/.zad/projects/<slug>/services/gcal/permissions.toml`

Both files apply simultaneously — a call must pass every file that
exists, and a missing file contributes no restrictions. This makes
it safe to ship a strict global baseline. See
[`examples/gcal-permissions/`](../examples/gcal-permissions/) for a
worked example and [`man/gcal.md`](../man/gcal.md) for the per-verb
reference. The key schema elements:

| Top-level | Applies to |
|---|---|
| `[content]` | `deny_words` / `deny_patterns` / `max_length` against event summary + description |
| `[time]` | UTC days + `HH:MM-HH:MM` windows — every verb |

Per-verb blocks: `[list_calendars]`, `[get_calendar]`,
`[list_events]`, `[get_event]`, `[create_event]`, `[update_event]`,
`[delete_event]`, `[invite]`, `[remind]`. Each accepts:

| Field | Shape | Meaning |
|---|---|---|
| `calendars` | `{ allow, deny }` pattern list | Gate the target calendar |
| `attendees` | `{ allow, deny }` pattern list | Gate each attendee email (writes only) |
| `content` | content rules | Narrow the top-level `[content]` defaults |
| `time` | time window | Narrow the top-level `[time]` defaults |
| `send_updates_allowed` | pattern list over `"none"`/`"external"`/`"all"` | Gate the `--send-updates` value |
| `max_future_days` | integer | Refuse events starting further than N days out |
| `min_notice_minutes` | integer | Refuse events starting in less than N minutes |
| `max_attendees` | integer | Cap attendee count after the write |
| `block_shared_calendars` | bool | Refuse writes on calendars where `accessRole != "owner"` |

Numeric caps intersect across layers via `min()`; boolean caps via
`OR` (strictest wins). Reminders are additionally capped at **40320
minutes (four weeks)** by a built-in, non-configurable rule.

## Spotify service (`spotify`)

Commands that drive it (documented in [`man/service.md`](../man/service.md) and [`man/spotify.md`](../man/spotify.md)):

- `zad service create spotify [--local]` — register OAuth credentials
  (interactive browser flow or `--refresh-token`).
- `zad service enable spotify` — enable the service in the current project.
- `zad service disable spotify` — disable it again (leaves credentials intact).

Every command accepts `--json` for script-friendly structured output.

### Credentials file

Stored at **one** of:

- Global: `~/.zad/services/spotify/config.toml`
- Local: `~/.zad/projects/<slug>/services/spotify/config.toml`

The project-local file wins over the global one for that project. The
format is flat:

```toml
scopes           = ["search", "playlists.read", "playlists.write", "library.read"]
default_playlist = "zad-test"     # optional
self_user_id     = "fakeuser123"  # optional — user ID captured at create time
```

| Key | Type | Default | Description |
|---|---|---|---|
| `scopes` | `[string]` | `["search", "playlists.read", "playlists.write", "library.read"]` | Capabilities the service is permitted to use. |
| `default_playlist` | string? | — | Optional default playlist for verbs that omit `--playlist`. Accepts a Spotify playlist ID, a `spotify:playlist:<id>` URI, or a directory alias. |
| `self_user_id` | string? | — | The authenticated user's Spotify user ID. Captured from `GET /me` during `service create` (or set via `--self-user`); required when creating a playlist (the API takes a user ID in the path). |

Spotify uses a **PKCE-only public client** — no `client_secret` is
issued or accepted by Spotify for desktop / CLI apps. `spotify` stores
**two** keychain entries per scope. Access tokens are not persisted:
each CLI invocation refreshes once and uses the token for that
process lifetime.

| Keychain account | Contents |
|---|---|
| `spotify-client-id:<scope>` | OAuth client ID. |
| `spotify-refresh:<scope>` | Refresh token. |

Scopes are **enforced at runtime, before any network call**. The
supported values are:

| Scope | Gates | Spotify OAuth scope |
|---|---|---|
| `search` | `search` | none (any authorized session works) |
| `playlists.read` | `playlists list`, `playlists show` | `playlist-read-private`, `playlist-read-collaborative` |
| `playlists.write` | `playlists create`, `rename`, `delete`, `add`, `remove` | `playlist-modify-private`, `playlist-modify-public` (plus the read scopes) |
| `library.read` | `library tracks list`, `library albums list` | `user-library-read` |
| `library.write` | `library tracks save/unsave`, `library albums save/unsave` | `user-library-modify` (plus `user-library-read`) |

When both a global and a project-local credentials file exist, the
local file **replaces** the global one for that project — scopes are
not merged. Write the full scope set each time.

### Creating the Spotify app

`zad service create spotify` expects a Spotify Developer **app** with
a registered redirect URI:

1. Visit <https://developer.spotify.com/dashboard>.
2. Click **Create app**. Name and description are arbitrary.
3. Under **Redirect URIs**, add `http://127.0.0.1` and save. zad's
   loopback listener picks a random port; Spotify accepts any port on
   `127.0.0.1` once the host is registered.
4. Copy the Client ID. `zad service create spotify` prompts for it,
   then opens your browser to authorize, captures the code on
   `http://127.0.0.1:<port>` (PKCE S256, random 32-byte state), and
   stores the returned refresh token.

The Spotify dashboard *also* shows a Client Secret — **you do not
need it**. zad stores only the Client ID + the refresh token. For CI
or non-interactive use, mint a refresh token out-of-band and pass
`--client-id` / `--refresh-token`.

### Permissions file

Scopes answer "is this family of operations enabled at all?".
Permissions are a second, finer layer — *which playlists, tracks,
albums, or queries; at what time, with what content* — and live in
an optional TOML file next to the credentials:

- Global: `~/.zad/services/spotify/permissions.toml`
- Local: `~/.zad/projects/<slug>/services/spotify/permissions.toml`

Both files apply simultaneously — a call must pass every file that
exists, and a missing file contributes no restrictions. See
[`examples/spotify-permissions/`](../examples/spotify-permissions/)
for a worked example and [`man/spotify.md`](../man/spotify.md) for
the per-verb reference. The key schema elements:

| Top-level | Applies to |
|---|---|
| `[content]` | `deny_words` / `deny_patterns` / `max_length` against playlist names, descriptions, and search queries |
| `[time]` | UTC days + `HH:MM-HH:MM` windows — every verb |

Per-verb blocks: `[search]`, `[playlists_read]`, `[playlists_write]`,
`[library_read]`, `[library_write]`. Each accepts:

| Field | Shape | Meaning |
|---|---|---|
| `targets` | `{ allow, deny }` pattern list | Gate the thing being acted on (playlist name/ID/URI, track/album URI, or query string) |
| `content` | content rules | Narrow the top-level `[content]` defaults |
| `time` | time window | Narrow the top-level `[time]` defaults |

Deny always wins over allow; an empty allow list contributes no
positive constraint (only the deny list applies).

## 1Password service (`1pass`)

Commands that drive it (documented in [`man/service.md`](../man/service.md) and [`man/1pass.md`](../man/1pass.md)):

- `zad service create 1pass [--local]` — register a Service Account
  token.
- `zad service enable 1pass` — enable the service in the current project.
- `zad service disable 1pass` — disable it again (leaves credentials intact).

Every command accepts `--json` for script-friendly structured output.

### Credentials file

Stored at **one** of:

- Global: `~/.zad/services/1pass/config.toml`
- Local:  `~/.zad/projects/<slug>/services/1pass/config.toml`

```toml
account       = "my.1password.com"
scopes        = ["read", "write"]
default_vault = "AgentWork"        # optional
```

| Key | Type | Default | Description |
|---|---|---|---|
| `account` | string | — | 1Password sign-in address. Passed to `op` as `OP_ACCOUNT`. |
| `scopes` | `[string]` | `["read"]` | Capabilities the service is permitted to use. |
| `default_vault` | string? | — | Optional default vault for verbs that omit `--vault`. |

A single keychain entry holds the Service Account token:

| Keychain account | Contents |
|---|---|
| `1pass-service-account:<scope>` | `ops_…` Service Account token. |

Scopes are **enforced at runtime, before any `op` spawn**:

| Scope | Gates |
|---|---|
| `read`  | `vaults`, `items`, `tags`, `get`, `read`, `inject`, `whoami` |
| `write` | `create` |

### Permissions file

Optional, located at:

- Global: `~/.zad/services/1pass/permissions.toml`
- Local:  `~/.zad/projects/<slug>/services/1pass/permissions.toml`

Both files intersect; local can only tighten global. Read-side axes
(`vaults`, `tags`, `items`, `categories`, `fields`) are **filters** —
anything out of scope is presented as if it doesn't exist, and
`get`/`read` on a hidden target return `op`'s own "no item found"
shape. See
[`examples/1pass-permissions/`](../examples/1pass-permissions/) for a
worked example and [`man/1pass.md`](../man/1pass.md) for the per-verb
reference.

| Top-level axis | Applies to | Gate |
|---|---|---|
| `vaults` | all read-side verbs | vault name or UUID |
| `tags`   | all read-side verbs | item tags |
| `items`  | all read-side verbs | item title or UUID |
| `categories` | all read-side verbs | `Login`, `API Credential`, `Secure Note`, … |
| `fields` | `get`, `read`, `inject` | field label or ID |
| `content` | `inject` output | body of the rendered template |
| `time`   | all | UTC days + `HH:MM-HH:MM` windows |

Per-verb blocks (`[vaults]`, `[items]`, `[tags]`, `[get]`, `[read]`,
`[inject]`) override the corresponding top-level axis.

The `[create]` block is **deny-by-default** — without an explicit
`[create].vaults.allow` entry matching the target vault, every
`create` call is rejected with `PermissionDenied`:

```toml
[create.vaults]
allow = ["AgentWork"]
[create.categories]
allow = ["Login", "API Credential", "Secure Note"]
[create.tags]
allow = ["agent-managed"]   # every created item must carry this tag
```

## Signed permission files

Every `permissions.toml` carries a top-level `[signature]` block
populated by `zad <svc> permissions init`. Load-time verification
fails closed — a missing, malformed, or key-mismatched signature
returns `PermissionDenied`. Do **not** edit the `[signature]` block
by hand; edit the policy fields and re-sign (for now,
`zad <svc> permissions init --force` regenerates the signature; PR 2
adds a dedicated `permissions sign` subcommand). See
[`docs/permissions.md`](permissions.md) for the trust model, keychain
entry name (`signing:v1`), and failure modes.

## Logging

zad always writes a rolling daily log file at a platform-appropriate
state directory (per `OSS_SPEC.md` §19.2):

| Platform | Path |
|---|---|
| Linux   | `~/.local/state/zad/debug.log` |
| macOS   | `~/Library/Application Support/zad/debug.log` |
| Windows | `%LOCALAPPDATA%\zad\debug.log` |

The global `--debug` flag additionally mirrors the log to stderr.
