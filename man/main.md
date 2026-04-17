# zad

> A Rust CLI that connects AI agents to external services (Discord, GitHub, Slack, etc.) via scoped service configurations instead of MCP servers.

## Synopsis

```
zad [OPTIONS] [COMMAND]
```

## Description

`zad` connects AI agents to external services (Discord, GitHub, Slack,
…) through scoped *services* instead of per-agent MCP servers. Each
service has credentials stored either globally
(`~/.zad/services/<service>/config.toml`, shared across every project)
or locally (`~/.zad/projects/<slug>/services/<service>/config.toml`,
scoped to one project), with `<slug>` being the absolute current
working directory with every `/` (and on Windows every `\` or `:`)
replaced by `-` — the same scheme Claude Code uses.

Bot tokens, API keys, and other secrets always live in the OS keychain
and are **never** written to the TOML.

Six actions operate on services:

- `zad service create <service>` — stores credentials for the service.
  Defaults to the global location; pass `--local` to store them only
  for the current project.
- `zad service enable <service>` — enables the service in the current
  project, using whichever credentials `create` registered (local wins
  over global).
- `zad service disable <service>` — disables the service in the current
  project by removing its entry from the project config. Inverse of
  `enable`. Does not touch credentials.
- `zad service list` — prints a table of known services with the state
  of global credentials, project-local credentials, and project
  enablement.
- `zad service show <service>` — prints the effective configuration
  and both scopes' details (paths, application id, scopes, token
  presence, project enablement) without ever revealing the token.
- `zad service delete <service>` — removes the stored credentials at
  the chosen scope (global by default, `--local` for project-scoped)
  and clears the matching OS-keychain entry. Inverse of `create`.

Every command supports `--json` to emit machine-readable output
instead of the human-readable default.

This manpage documents every command the binary ships. Nested
subcommands are folded into the same page rather than split across
separate manpages.

## Top-level flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--version` | bool | false | Print version and exit. |
| `--help`    | bool | false | Print help and exit. |
| `--debug`   | bool | false | Enable debug-level logging on stderr. The on-disk log at `~/.local/state/zad/debug.log` (Linux) or `~/Library/Application Support/zad/debug.log` (macOS) is written regardless. |

## Subcommands

| Command | Description |
|---|---|
| `service <ACTION> <SERVICE>` | Configure or inspect external services. |
| `help` | Show help text. |

---

## `zad service`

```
zad service <ACTION> <SERVICE>
```

Configure or inspect external services. Actions:

| Action | Description |
|---|---|
| `create <service>` | Create credentials for the service. |
| `enable <service>` | Enable the service in the current project. |
| `disable <service>` | Disable the service in the current project (inverse of `enable`). |
| `list` | List all services with credential and project-enablement status. |
| `show <service>` | Show the effective configuration and both scopes' details. |
| `delete <service>` | Delete credentials for the service (inverse of `create`). |

| Service | Description |
|---|---|
| `discord` | Discord bot-token service. |

### `zad service create discord`

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

#### Flags

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
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

#### Recognised scopes

- `guilds` — list and read guilds the bot is a member of.
- `messages.read` — read channel history.
- `messages.send` — post messages to channels and DMs.
- `channels.manage` — create and delete channels.
- `gateway.listen` — subscribe to the real-time gateway.

### `zad service enable discord`

```
zad service enable discord [OPTIONS]
```

Enables the Discord service in the current project by writing
`[service.discord] enabled = true` to
`~/.zad/projects/<slug>/config.toml`. Requires credentials registered
via `zad service create discord` (local credentials under the project
slug win over global ones). The project config **never** contains
credentials.

#### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--force` | bool | `false` | Overwrite an existing `[service.discord]` entry in the project config. |
| `--non-interactive` | bool | `false` | Reserved: `enable` has no prompts today. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

#### Notes

- The **MESSAGE_CONTENT** privileged intent must be enabled on the bot
  in the Discord developer portal for the `body` field on incoming
  gateway `MessageCreated` events to contain text; without it Discord
  delivers empty content for guild messages.

### `zad service disable discord`

```
zad service disable discord [OPTIONS]
```

Disables the Discord service in the current project by removing the
`[service.discord]` entry from `~/.zad/projects/<slug>/config.toml`.
This is the inverse of `zad service enable discord`. It does **not**
delete credentials — use `zad service delete discord` for that.

#### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--force` | bool | `false` | Succeed silently when the service is not currently enabled in this project. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

### `zad service list`

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

#### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--json` | bool | `false` | Emit machine-readable JSON instead of the human-readable table. |

### `zad service show discord`

```
zad service show discord [OPTIONS]
```

Prints the effective Discord configuration (local wins over global)
plus a per-scope block with the config-file path, application ID,
selected scopes, optional default guild, and token presence in the OS
keychain. The bot token itself is **never** printed. Exits 0 even
when nothing is configured — output simply reports "no credentials".

#### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

### `zad service delete discord`

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

#### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--local` | bool | `false` | Delete the project-scoped credentials under `~/.zad/projects/<slug>/services/discord/` instead of the global ones. |
| `--force` | bool | `false` | Succeed silently when no config file exists at the chosen scope. Keychain deletion is always idempotent. |
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. |

---

## Environment variables

| Variable | Description |
|---|---|
| Value of `--bot-token-env` | Source of the Discord bot token. Never logged or written to the TOML. |
| `ZAD_HOME_OVERRIDE` | Override `~/` when resolving `~/.zad/`. Tests only. |
| `ZAD_SECRETS_MEMORY` | When `1`, store secrets in a process-local map instead of the OS keychain. Tests only. |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | Generic error — token validation failed, keyring write failed, filesystem error, etc. |
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
zad service delete discord --local     # remove this project's local creds only
zad service delete discord             # remove the global creds (keychain too)

# Script-friendly JSON output is available on every command
zad service list --json | jq '.services[] | select(.enabled)'
```

## See also

- [`docs/configuration.md`](../docs/configuration.md) — config file reference.
- [`docs/architecture.md`](../docs/architecture.md) — module layout.
