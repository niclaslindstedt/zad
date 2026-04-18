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

## Top-level flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--version` | bool | false | Print version and exit. |
| `--help`    | bool | false | Print help and exit. |
| `--debug`   | bool | false | Enable debug-level logging on stderr. The on-disk log at `~/.local/state/zad/debug.log` (Linux) or `~/Library/Application Support/zad/debug.log` (macOS) is written regardless. |
| `--help-agent` | bool | false | Print a compact, prompt-injectable description of the CLI — its commands, most important flags and env vars, and pointers to the `commands`, `man`, and `docs` discovery surfaces. Designed for splicing into an agent prompt via command substitution (`$(zad --help-agent)`). See `OSS_SPEC.md` §12.1. |
| `--debug-agent` | bool | false | Print a troubleshooting block (log paths, config precedence, env vars, diagnostic commands, version). See `OSS_SPEC.md` §12.2. |

## Subcommands

| Command | Description | Manpage |
|---|---|---|
| `service <ACTION> <SERVICE>` | Configure or inspect external services (credentials, project enablement). | [`zad man service`](service.md) |
| `discord <VERB>` | Operate the Discord service at runtime (send, read, channels, join, leave, discover, directory, permissions). | [`zad man discord`](discord.md) |
| `telegram <VERB>` | Operate the Telegram service at runtime (send, read, chats, discover, directory, permissions). | [`zad man telegram`](telegram.md) |
| `commands [NAME]...` | Enumerate every CLI command, flag, and realistic example; also emits a JSON dump consumed by the website extractor. | [`zad man commands`](commands.md) |
| `docs [TOPIC]` | Print topic documentation (`docs/*.md`) embedded in the binary at build time. | [`zad man docs`](docs.md) |
| `man [COMMAND]` | Print reference manpages (`man/*.md`) embedded in the binary at build time. | [`zad man man`](man.md) |

Each top-level command has its own manpage with every subcommand, flag,
example, and exit code. This page only sketches the cross-cutting
surface.

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
| 1 | Generic error — token validation failed, keyring write failed, filesystem error, API failure. |
| 2 | Usage error — conflicting flags, invalid numeric ID, unknown scope, missing subcommand. |

## Examples

```sh
# One-shot: register Discord creds globally, enable in this project, discover names,
# and post a message — using the directory instead of snowflakes.
export DISCORD_BOT_TOKEN=...
zad service create discord \
    --application-id 1234567890 \
    --bot-token-env DISCORD_BOT_TOKEN \
    --scopes guilds,messages.send \
    --non-interactive
zad service enable discord
zad discord discover
zad discord send --channel general "deploy finished"

# Script-friendly JSON output is available on every command
zad service list --json | jq '.services[] | select(.enabled)'

# Prime an agent with the whole CLI surface in one prompt
claude "Help me automate X $(zad --help-agent)"
```

## See also

- [`zad man service`](service.md) — credential management and project enablement.
- [`zad man discord`](discord.md) — runtime verbs for the Discord service.
- [`zad man telegram`](telegram.md) — runtime verbs for the Telegram service.
- [`docs/configuration.md`](../docs/configuration.md) — config file reference.
- [`docs/architecture.md`](../docs/architecture.md) — module layout.
