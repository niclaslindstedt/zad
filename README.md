# zad

A Rust CLI that connects AI agents to external services (Discord, GitHub, Slack, etc.) via scoped service configurations instead of MCP servers.

[![CI](https://github.com/niclaslindstedt/zad/actions/workflows/ci.yml/badge.svg)](https://github.com/niclaslindstedt/zad/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![crates.io](https://img.shields.io/crates/v/zad.svg)](https://crates.io/crates/zad)

## Why?

- Services replace per-agent MCP server setup — one config file wires a service to any agent
- Permission files enforce fine-grained scopes (time windows, content filters) beyond what the upstream API offers
- `--help-agent` flag emits machine-readable docs so an LLM can configure services on the user's behalf
- Global (~/.zad/) and project-local configs let teams share defaults while overriding per-repo
- Extending zad with a new provider is a single Rust trait implementation; hooking up services is pure TOML config


## Prerequisites

- Rust **1.88+** (edition 2024) with `cargo`.
- An OS keychain zad can write to: macOS Keychain, Linux Secret Service
  (gnome-keyring, KWallet, …), or Windows Credential Manager.

## Install

```sh
cargo install --path .
```

## Quick start

Two steps: register credentials once, then enable the service per project.

```sh
# 1. Register global Discord credentials (one-time). Interactive:
#    zad opens your browser to the Developer Portal bot page,
#    you hit "Reset Token" → "Copy", paste once.
zad service create discord --application-id 1234567890

# After create succeeds, zad also opens the OAuth install URL so you
# can add the bot to a guild.

# 2. Enable the service inside each project that should use it.
cd ~/code/my-project
zad service enable discord

# 3. Populate the name -> snowflake directory so you can use channel
#    and user names instead of pasting 19-digit IDs.
zad discord discover

# 4. Drive the service at runtime.
zad discord send --channel general "deploy finished"
zad discord read --channel general --limit 20
zad discord channels --json
```

For headless / CI setups, pass the token non-interactively:

```sh
export DISCORD_BOT_TOKEN=...   # from https://discord.com/developers
zad service create discord \
    --application-id 1234567890 \
    --bot-token-env DISCORD_BOT_TOKEN \
    --scopes guilds,messages.send \
    --no-browser --non-interactive
```

Use `--local` on `create` to store credentials only for the current
project (under `~/.zad/projects/<slug>/services/discord/`).

## Usage

```
zad service <ACTION> <SERVICE>   # configuration (create / enable / list / …)
zad <SERVICE> <VERB>             # runtime operations (service-specific verbs)
```

Configuration actions: `create` (register credentials), `enable` /
`disable` (toggle for this project), `list`, `show`, `status` (ping
the provider to confirm credentials actually work), and `delete`.
`zad status` (top-level) runs `status` across every service at once
and is designed for agents — `--json` emits a stable envelope and the
exit code reflects whether every configured service pinged
successfully.

Runtime verbs are chosen per service.

- **`discord`**: `send`, `read`, `channels`, `join`, `leave`, plus
  `discover` (best-effort walk that caches a name → snowflake map at
  `~/.zad/projects/<slug>/services/discord/directory.toml`),
  `directory` (list / set / remove entries by hand), and `permissions`
  (inspect, scaffold, or dry-run the per-project permissions policy).
  After `discover`, the destination flags accept names —
  `--channel general`, `--dm @alice` — with a numeric snowflake still
  working as a fallback. Mutating verbs (`send`, `join`, `leave`) take
  `--dry-run`, which previews the outgoing call — scope and permission
  checks still fire, but no bot token is loaded and no network
  request is made.
- **`telegram`**: `send`, `read`, `chats`, `discover`, `directory`,
  and `permissions`. `--chat` accepts a signed `chat_id`
  (negative for groups/supergroups), a `@username` for public
  channels, or a directory alias seeded by `discover`. `send` takes
  `--dry-run` with the same semantics as Discord's.

Every command takes `--json` for machine-readable output.

Today the shipped services are `discord` and `telegram`. See
[`man/main.md`](man/main.md) for the top-level overview and
[`man/service.md`](man/service.md), [`man/discord.md`](man/discord.md),
and [`man/telegram.md`](man/telegram.md) for the full per-command
reference.

### Permissions (optional second layer)

Scopes declare *which families of operations* a service may perform;
**permissions** are a finer layer that pins down *which channels, which
users, which times, and which content* each function is allowed to
touch. They live in an optional TOML file — globally at
`~/.zad/services/<service>/permissions.toml` and/or per project at
`~/.zad/projects/<slug>/services/<service>/permissions.toml`. Both
files apply simultaneously (strictest wins), so a global baseline can
never be loosened by a project. An absent file contributes no
restrictions.

```sh
# Scaffold a project-local policy (deny admin-like channels + channels.manage).
zad discord permissions init --local

# Dry-run an action without hitting Discord.
zad discord permissions check --function send --channel general --body "hello"
```

See [`docs/configuration.md`](docs/configuration.md#permissions-file)
for the full schema. The same pattern will apply to every future
service — each provider picks up the generic `content` / `time` /
`allow` / `deny` primitives and names its own per-function blocks.

## Configuration

See [`docs/configuration.md`](docs/configuration.md) for the full list of
config keys and secret-storage details. The short version:

- Config lives at `~/.zad/projects/<slug>/config.toml`.
- Bot tokens and other secrets live in the OS keychain, never in TOML.
- Override `~/` with `ZAD_HOME_OVERRIDE` for tests.

## Examples

See [`examples/`](examples/) for runnable demos.

## Troubleshooting

**Keychain permission denied** — On macOS, `zad` writes to the system keychain.
If you see `Error: keychain access denied`, open Keychain Access, find the
`zad` entry, and grant access; or re-run with `sudo` once to seed the entry.

**Missing `DISCORD_BOT_TOKEN`** — `zad service create discord` reads this
variable from the environment. Export it before running the command:
```sh
export DISCORD_BOT_TOKEN=<your-bot-token>
```
If you pass `--bot-token-env` with a custom variable name, export that name
instead.

**`zad: command not found` after `cargo install`** — Ensure `~/.cargo/bin` is
on your `PATH`. Add `export PATH="$HOME/.cargo/bin:$PATH"` to your shell
profile and reload it.

## Documentation

- [Getting started](docs/getting-started.md)
- [Configuration](docs/configuration.md)
- [Architecture](docs/architecture.md)
- [Troubleshooting](docs/troubleshooting.md)

## Community

- **Bugs and feature requests** — [GitHub issues](https://github.com/niclaslindstedt/zad/issues).
- **Questions, ideas, show-and-tell** — [GitHub Discussions](https://github.com/niclaslindstedt/zad/discussions).
- **Security reports** — private, via the channel in [SECURITY.md](SECURITY.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under [MIT](LICENSE).
