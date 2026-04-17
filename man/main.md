# zad

> A Rust CLI that connects AI agents to external services (Discord, GitHub, Slack, etc.) via scoped adapter configurations instead of MCP servers.

## Synopsis

```
zad [OPTIONS] [COMMAND]
```

## Description

`zad` connects AI agents to external services (Discord, GitHub, Slack,
…) through scoped *adapters* instead of per-agent MCP servers. Each
adapter has credentials stored either globally
(`~/.zad/adapters/<adapter>/config.toml`, shared across every project)
or locally (`~/.zad/projects/<slug>/adapters/<adapter>/config.toml`,
scoped to one project), with `<slug>` being the absolute current
working directory with every `/` (and on Windows every `\` or `:`)
replaced by `-` — the same scheme Claude Code uses.

Bot tokens, API keys, and other secrets always live in the OS keychain
and are **never** written to the TOML.

Two actions operate on adapters:

- `zad adapter create <adapter>` — stores credentials for the adapter.
  Defaults to the global location; pass `--local` to store them only
  for the current project.
- `zad adapter add <adapter>` — enables the adapter in the current
  project, using whichever credentials `create` registered (local wins
  over global).

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
| `adapter <ACTION> <ADAPTER>` | Configure or inspect external-service adapters. |
| `help` | Show help text. |

---

## `zad adapter`

```
zad adapter <ACTION> <ADAPTER>
```

Configure or inspect external-service adapters. Actions:

| Action | Description |
|---|---|
| `create <adapter>` | Create credentials for the adapter. |
| `add <adapter>` | Enable the adapter in the current project. |

| Adapter | Description |
|---|---|
| `discord` | Discord bot-token adapter. |

### `zad adapter create discord`

```
zad adapter create discord [--local] [OPTIONS]
```

Interactively (or via flags) collects the Discord application ID, bot
token, default guild, and capability scopes; validates the token against
Discord's `GET /users/@me` endpoint; stores the token in the OS
keychain; and writes a flat config file to either
`~/.zad/adapters/discord/config.toml` (global, the default) or
`~/.zad/projects/<slug>/adapters/discord/config.toml` (with `--local`).

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

#### Recognised scopes

- `guilds` — list and read guilds the bot is a member of.
- `messages.read` — read channel history.
- `messages.send` — post messages to channels and DMs.
- `channels.manage` — create and delete channels.
- `gateway.listen` — subscribe to the real-time gateway.

### `zad adapter add discord`

```
zad adapter add discord [OPTIONS]
```

Enables the Discord adapter in the current project by writing
`[adapter.discord] enabled = true` to
`~/.zad/projects/<slug>/config.toml`. Requires credentials registered
via `zad adapter create discord` (local credentials under the project
slug win over global ones). The project config **never** contains
credentials.

#### Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--force` | bool | `false` | Overwrite an existing `[adapter.discord]` entry in the project config. |
| `--non-interactive` | bool | `false` | Reserved: `add` has no prompts today. |

#### Notes

- The **MESSAGE_CONTENT** privileged intent must be enabled on the bot
  in the Discord developer portal for the `body` field on incoming
  gateway `MessageCreated` events to contain text; without it Discord
  delivers empty content for guild messages.

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
zad adapter create discord \
    --application-id 1234567890 \
    --bot-token-env DISCORD_BOT_TOKEN \
    --scopes guilds,messages.send \
    --non-interactive

# Use project-specific credentials instead
zad adapter create discord --local --application-id 1234 --bot-token-env DISCORD_BOT_TOKEN

# Enable the adapter in this project
zad adapter add discord

# Rotate the global token in place
zad adapter create discord --force --bot-token-env DISCORD_BOT_TOKEN_NEW
```

## See also

- [`docs/configuration.md`](../docs/configuration.md) — config file reference.
- [`docs/architecture.md`](../docs/architecture.md) — module layout.
