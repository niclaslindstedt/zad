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
| `status <service>` | Check whether credentials work by pinging the provider. |
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

## `zad service status discord`

```
zad service status discord [--json]
```

Verifies that the effective Discord credentials work end-to-end by
calling `GET /users/@me` with the stored bot token. Reports, per scope:
the config path, whether a config file exists, whether the token is
present in the OS keychain, and — for the effective scope only — the
live ping result (`ok` with the authenticated bot username, or
`FAILED` with the provider error message).

Exits `0` when the effective scope's ping succeeds; exits `1` when
the effective scope fails or no credentials are configured at all.
Designed for agents: pair `--json` with `$?` to branch on the outcome
without parsing.

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. Recommended for agents. |

### JSON shape

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

`effective` is omitted (null) when no scope is configured. `check`
appears only on the effective scope; non-effective scopes report
presence without a live ping to avoid doubling provider rate-limit
cost.

## `zad service status telegram`

```
zad service status telegram [--json]
```

Same shape as `status discord`, pinging Telegram's `getMe` endpoint.

### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. Recommended for agents. |

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
zad service status discord             # ping the provider with the effective token
zad service delete discord --local     # remove this project's local creds only
zad service delete discord             # remove the global creds (keychain too)

# Script-friendly JSON output is available on every command
zad service list --json | jq '.services[] | select(.enabled)'
zad service status discord --json | jq '.ok'
```

## See also

- [`zad man discord`](discord.md) — runtime verbs for the Discord service.
- [`zad man telegram`](telegram.md) — runtime verbs for the Telegram service.
- [`zad man main`](main.md) — top-level CLI overview.
- [`docs/configuration.md`](../docs/configuration.md) — config file reference.
