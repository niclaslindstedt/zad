# zad service

> Configure or inspect external services (credentials + project enablement).

## Synopsis

```
zad service <ACTION> [<SERVICE>] [OPTIONS]
```

## Description

`zad service` manages the credentials and project-enablement state for
every external service zad integrates with. Credentials are stored
either globally (`~/.zad/services/<service>/config.toml`, shared across
every project) or locally
(`~/.zad/projects/<slug>/services/<service>/config.toml`, scoped to one
project), with `<slug>` being the absolute current working directory
with every `/` (and on Windows every `\` or `:`) replaced by `-` — the
same scheme Claude Code uses.

Bot tokens, API keys, and other secrets always live in the OS keychain
and are **never** written to the TOML.

Seven actions operate on services:

| Action | Description |
|---|---|
| `create <service>` | Create credentials for the service. |
| `enable <service>` | Enable the service in the current project. |
| `disable <service>` | Disable the service in the current project (inverse of `enable`). |
| `list` | List all services with credential and project-enablement status. |
| `show <service>` | Show the effective configuration and both scopes' details. |
| `status [--service <name>]` | Check whether credentials work by pinging the provider. Without `--service`, every service in the registry is pinged in parallel. |
| `delete <service>` | Delete credentials for the service (inverse of `create`). |

Recognised services:

| Service | Description |
|---|---|
| `discord` | Discord bot-token service. See `zad man discord` for the runtime verbs. |
| `telegram` | Telegram bot-token service. See `zad man telegram` for the runtime verbs. |

Every command supports `--json` to emit machine-readable output
instead of the human-readable default.

## `zad service create discord`

```
zad service create discord [--local] [OPTIONS]
```

Interactively (or via flags) collects the Discord application ID, bot
token, default guild, and capability scopes; validates the token against
Discord's `GET /users/@me` endpoint; stores the token in the OS
keychain; and writes a flat config file to either
`~/.zad/services/discord/config.toml` (global, the default) or
`~/.zad/projects/<slug>/services/discord/config.toml` (with `--local`).

The token is stored at keychain `service="zad"`, `account="discord-bot:global"`
for global credentials and `"discord-bot:<slug>"` for local ones.

When neither `--bot-token` nor `--bot-token-env` is supplied, the
interactive prompt prints the deep link to your application's
developer-portal bot page
(`https://discord.com/developers/applications/<app_id>/bot`) and opens
it in your browser so you can hit "Reset Token" → "Copy" and paste
once. Pass `--no-browser` to skip the browser open (the URL is still
printed). Discord doesn't issue bot tokens via OAuth — this flow is
just a convenience around the portal step.

After a successful create, the OAuth bot-install URL
(`https://discord.com/api/oauth2/authorize?client_id=<app_id>&scope=bot&permissions=0`)
is printed and (unless `--no-browser`) opened in your browser so you
can add the bot to a guild.

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--local` | bool | `false` | Store credentials under the project slug instead of the shared global location. |
| `--application-id <id>` | string | — | Discord application (bot) ID. |
| `--bot-token <token>` | string | — | Bot token. Stored in the OS keychain, not the TOML. |
| `--bot-token-env <VAR>` | string | — | Read the bot token from the named environment variable. Mutually exclusive with `--bot-token`. |
| `--default-guild <id>` | string | — | Optional default guild (server) ID. |
| `--self-user <id>` | string | — | Your own Discord user ID (numeric snowflake). Stored non-secretly as `self_user_id` and resolved from `@me` in later `send --dm` targets. Validated against `GET /users/{id}` before being persisted. Interactive mode prints the Developer Mode recipe before the prompt; omit this flag and leave the prompt blank to skip — the field can be set later via `zad discord self set <id>`. |
| `--scopes <list>` | CSV | `guilds,messages.read,messages.send` | Capabilities to enable. |
| `--force` | bool | `false` | Overwrite any existing credentials at the chosen scope. |
| `--non-interactive` | bool | `false` | Fail instead of prompting for any missing value. |
| `--no-validate` | bool | `false` | Skip the `GET /users/@me` token validation step. |
| `--no-browser` | bool | `false` | Don't auto-open the developer-portal URL or the post-create install URL in the system browser. URLs are still printed. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

### Recognised scopes

- `guilds` — list and read guilds the bot is a member of.
- `messages.read` — read channel history.
- `messages.send` — post messages to channels and DMs.
- `channels.manage` — create and delete channels.
- `gateway.listen` — subscribe to the real-time gateway.

## `zad service create telegram`

```
zad service create telegram [--local] [OPTIONS]
```

Interactively (or via flags) collects the Telegram bot token,
optional default chat, and capability scopes; validates the token
against Telegram's `getMe` endpoint; stores the token in the OS
keychain; and writes a flat config file to either
`~/.zad/services/telegram/config.toml` (global, the default) or
`~/.zad/projects/<slug>/services/telegram/config.toml` (with
`--local`).

The token is stored at keychain `service="zad"`,
`account="telegram-bot:global"` for global credentials and
`"telegram-bot:<slug>"` for local ones. Telegram bots carry their
identity inside the bot token itself, so there is no separate
`application_id` field.

Bot tokens come from a chat with `@BotFather` on Telegram, not a web
page, so `create telegram` uses a plain password prompt instead of the
browser-deep-link flow `create discord` uses.

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--local` | bool | `false` | Store credentials under the project slug instead of the shared global location. |
| `--bot-token <token>` | string | — | Bot token. Stored in the OS keychain, not the TOML. |
| `--bot-token-env <VAR>` | string | — | Read the bot token from the named environment variable. Mutually exclusive with `--bot-token`. |
| `--default-chat <ref>` | string | — | Optional default chat. Accepts a signed chat_id (negative for groups/supergroups), a `@username` (public channels/supergroups), or a directory alias. |
| `--self-chat <chat_id>` | signed integer | — | Your own private-chat ID with this bot. Stored non-secretly as `self_chat_id` and resolved from `@me` in later `send`/`read` targets. Interactive mode skips this flag and instead offers to run the capture flow — polling `getUpdates` for up to 60s waiting for your first message to the bot. Non-interactive mode writes the flag verbatim. The field can be set later via `zad telegram self capture` or `zad telegram self set <id>`. |
| `--scopes <list>` | CSV | `chats,messages.read,messages.send` | Capabilities to enable. |
| `--force` | bool | `false` | Overwrite any existing credentials at the chosen scope. |
| `--non-interactive` | bool | `false` | Fail instead of prompting for any missing value. |
| `--no-validate` | bool | `false` | Skip the `getMe` token validation step. |
| `--no-browser` | bool | `false` | Reserved — Telegram's `create` never opens a browser, so the flag is a no-op for this service. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

### Recognised scopes

- `chats` — list and discover chats the bot has seen.
- `messages.read` — read buffered chat history.
- `messages.send` — post messages to chats.
- `gateway.listen` — subscribe to the update stream (library-level only).

## `zad service enable discord`

```
zad service enable discord [OPTIONS]
```

Enables the Discord service in the current project by writing
`[service.discord] enabled = true` to
`~/.zad/projects/<slug>/config.toml`. Requires credentials registered
via `zad service create discord` (local credentials under the project
slug win over global ones). The project config **never** contains
credentials.

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--force` | bool | `false` | Overwrite an existing `[service.discord]` entry in the project config. |
| `--non-interactive` | bool | `false` | Reserved: `enable` has no prompts today. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

### Notes

- The **MESSAGE_CONTENT** privileged intent must be enabled on the bot
  in the Discord developer portal for the `body` field on incoming
  gateway `MessageCreated` events to contain text; without it Discord
  delivers empty content for guild messages.

## `zad service enable telegram`

```
zad service enable telegram [OPTIONS]
```

Enables the Telegram service in the current project by writing
`[service.telegram] enabled = true` to
`~/.zad/projects/<slug>/config.toml`. Requires credentials registered
via `zad service create telegram` (local wins over global). The
project config **never** contains credentials.

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--force` | bool | `false` | Overwrite an existing `[service.telegram]` entry in the project config. |
| `--non-interactive` | bool | `false` | Reserved: `enable` has no prompts today. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

## `zad service disable discord`

```
zad service disable discord [OPTIONS]
```

Disables the Discord service in the current project by removing the
`[service.discord]` entry from `~/.zad/projects/<slug>/config.toml`.
This is the inverse of `zad service enable discord`. It does **not**
delete credentials — use `zad service delete discord` for that.

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--force` | bool | `false` | Succeed silently when the service is not currently enabled in this project. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

## `zad service disable telegram`

```
zad service disable telegram [OPTIONS]
```

Inverse of `zad service enable telegram`. Leaves credentials intact —
use `zad service delete telegram` to remove them as well.

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--force` | bool | `false` | Succeed silently when the service is not currently enabled in this project. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

## `zad service list`

```
zad service list [OPTIONS]
```

Prints a table showing, for every known service, whether global
credentials exist (`~/.zad/services/<service>/config.toml`), whether
local credentials exist for the current project's slug
(`~/.zad/projects/<slug>/services/<service>/config.toml`), and whether
the service is enabled in the current project's `config.toml`.

Output columns:

| Column | Values |
|---|---|
| `SERVICE` | Service name. |
| `GLOBAL`  | `yes` / `no`. |
| `LOCAL`   | `yes` / `no` (always relative to the current working directory's slug). |
| `PROJECT` | `enabled` / `disabled`. |

If nothing is configured anywhere, an explanatory hint is printed
after the table.

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--json` | bool | `false` | Emit machine-readable JSON instead of the human-readable table. |

## `zad service show discord`

```
zad service show discord [OPTIONS]
```

Prints the effective Discord configuration (local wins over global)
plus a per-scope block with the config-file path, application ID,
selected scopes, optional default guild, and token presence in the OS
keychain. The bot token itself is **never** printed. Exits 0 even
when nothing is configured — output simply reports "no credentials".

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

## `zad service show telegram`

```
zad service show telegram [OPTIONS]
```

Same shape as `show discord`, reporting the Telegram config's selected
scopes, optional `default_chat`, and token presence in the OS
keychain. The bot token itself is **never** printed.

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

## `zad service status`

```
zad service status [--service <NAME>] [--json]
```

Checks whether service credentials actually work by pinging the
provider. This is the agent-facing health check: one command covers
every service, and the JSON envelope is stable for script consumption.

Without `--service`, every service in zad's internal registry is
pinged in parallel (so adding a service doesn't linearly inflate
latency) and an aggregate envelope is emitted. With `--service`,
only the named service is pinged and the per-service envelope is
emitted.

Per service, the command:

1. Loads the global and local config files (if any).
2. Determines the *effective* scope (local wins over global).
3. Reads the secret for the effective scope out of the OS keychain.
4. Calls the provider's lightweight identity endpoint (Discord's
   `GET /users/@me`, Telegram's `getMe`). The identity the provider
   returns is reported as `authenticated_as`.

Only the effective scope is pinged — pinging both `global` and `local`
when both are configured would double the per-run provider rate-limit
cost. The non-effective scope is still reported (`configured`,
`credentials_present`) so an agent can see what's on disk without
spending a second API call.

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--service <NAME>` | enum | — | Limit the check to a single service (`discord`, `telegram`). Without this flag every service is pinged. Clap rejects unknown names with exit 2. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. Recommended for agents. |

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Every queried service's effective scope pinged successfully. Services with no credentials at all (`effective: null`) do **not** count as failures — "not configured" is different from "broken". |
| 1 | At least one queried service's effective scope failed (auth rejected, network error, missing keychain entry). |
| 2 | Usage error (unknown `--service` value, etc.). |

### JSON shape — aggregate (no `--service`)

```json
{
  "command": "service.status",
  "ok": true,
  "services": [
    {
      "service": "discord",
      "effective": "global",
      "ok": true,
      "global": {
        "path": "...",
        "configured": true,
        "credentials_present": true,
        "check": { "ok": true, "authenticated_as": "mybot" }
      },
      "local":  { "path": "...", "configured": false, "credentials_present": false },
      "project": { "config": "...", "enabled": true }
    },
    {
      "service": "telegram",
      "ok": false,
      "global": { "path": "...", "configured": true, "credentials_present": false },
      "local":  { "path": "...", "configured": false, "credentials_present": false },
      "project": { "config": "...", "enabled": false }
    }
  ]
}
```

### JSON shape — single service (`--service <NAME>`)

```json
{
  "command": "service.status.discord",
  "service": "discord",
  "effective": "global",
  "ok": true,
  "global": {
    "path": "...",
    "configured": true,
    "credentials_present": true,
    "check": { "ok": true, "authenticated_as": "mybot" }
  },
  "local":  { "path": "...", "configured": false, "credentials_present": false },
  "project": { "config": "...", "enabled": true }
}
```

Each entry in the aggregate's `services` array has the same shape as
the single-service envelope, minus the top-level `command` field
(that's hoisted out to the aggregate).

`effective` is omitted (null) when no scope is configured. `check`
appears only on the effective scope; non-effective scopes report
presence without a live ping.

### Examples

```sh
# Human-readable summary, one row per service
zad service status

# Agent use: JSON + exit code covers every service in one call
if zad service status --json > /tmp/zad-status.json; then
  echo "all good"
else
  jq '.services[] | select(.ok == false)' < /tmp/zad-status.json
fi

# Pluck just the working services
zad service status --json | jq '.services[] | select(.ok) | .service'

# Narrow to a single service (pings `GET /users/@me` for discord,
# `getMe` for telegram)
zad service status --service discord
zad service status --service discord --json | jq '.ok'
```

## `zad service delete discord`

```
zad service delete discord [OPTIONS]
```

Removes the Discord service's credentials at the chosen scope (global
by default, `--local` for project-scoped) and clears the matching OS
keychain entry (`discord-bot:global` or `discord-bot:<slug>`). This
is the inverse of `zad service create discord`. It does **not**
disable the service in the project's `config.toml`; if the project
still references the service a warning is printed (run
`zad service disable discord` to clear it).

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--local` | bool | `false` | Delete the project-scoped credentials under `~/.zad/projects/<slug>/services/discord/` instead of the global ones. |
| `--force` | bool | `false` | Succeed silently when no config file exists at the chosen scope. Keychain deletion is always idempotent. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

## `zad service delete telegram`

```
zad service delete telegram [OPTIONS]
```

Inverse of `zad service create telegram`. Removes the config file at
the chosen scope and clears the matching OS keychain entry
(`telegram-bot:global` or `telegram-bot:<slug>`). Does **not** disable
the service in the project's `config.toml` — run `zad service disable
telegram` to clear that as well.

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--local` | bool | `false` | Delete the project-scoped credentials under `~/.zad/projects/<slug>/services/telegram/` instead of the global ones. |
| `--force` | bool | `false` | Succeed silently when no config file exists at the chosen scope. Keychain deletion is always idempotent. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

## Environment variables

| Variable | Description |
|---|---|
| Value of `--bot-token-env` | Source of a service's secret (e.g. Discord bot token). Never logged or written to the TOML. |
| `ZAD_HOME_OVERRIDE` | Override `~/` when resolving `~/.zad/`. Tests only. |
| `ZAD_SECRETS_MEMORY` | When `1`, store secrets in a process-local map instead of the OS keychain. Tests only. |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | Generic error — token validation failed, keyring write failed, filesystem error. |
| 2 | Usage error — conflicting flags, invalid numeric ID, unknown scope. |

## Examples

```sh
# Register global Discord credentials once, reuse them across projects
export DISCORD_BOT_TOKEN=...
zad service create discord \
    --application-id 1234567890 \
    --bot-token-env DISCORD_BOT_TOKEN \
    --scopes guilds,messages.send \
    --non-interactive

# Use project-specific credentials instead
zad service create discord --local --application-id 1234 --bot-token-env DISCORD_BOT_TOKEN

# Enable the service in this project
zad service enable discord

# Disable it again (leaves credentials intact)
zad service disable discord

# Rotate the global token in place
zad service create discord --force --bot-token-env DISCORD_BOT_TOKEN_NEW

# Inspect and clean up
zad service list                       # see which services have creds / are enabled
zad service show discord               # show the effective config + both scopes
zad service status                     # ping every service in one go (agent-facing)
zad service status --service discord   # pin the check to a single service
zad service delete discord --local     # remove this project's local creds only
zad service delete discord             # remove the global creds (keychain too)

# Script-friendly JSON output is available on every command
zad service list --json | jq '.services[] | select(.enabled)'
zad service status --json | jq '.ok'
```

## See also

- [`zad man discord`](discord.md) — runtime verbs for the Discord service.
- [`zad man telegram`](telegram.md) — runtime verbs for the Telegram service.
- [`zad man main`](main.md) — top-level CLI overview.
- [`docs/configuration.md`](../docs/configuration.md) — config file reference.
