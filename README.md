# zad

A Rust CLI that connects AI agents to external services (Discord, GitHub, Slack, etc.) via scoped adapter configurations instead of MCP servers.

[![CI](https://github.com/niclaslindstedt/zad/actions/workflows/ci.yml/badge.svg)](https://github.com/niclaslindstedt/zad/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![crates.io](https://img.shields.io/crates/v/zad.svg)](https://crates.io/crates/zad)

## Why?

- Adapters replace per-agent MCP server setup — one config file wires a service to any agent
- Permission files enforce fine-grained scopes (time windows, content filters) beyond what the upstream API offers
- `--help-agent` flag emits machine-readable docs so an LLM can configure adapters on the user's behalf
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

Two steps: register credentials once, then enable the adapter per project.

```sh
# 1. Register global Discord credentials (one-time).
export DISCORD_BOT_TOKEN=...   # from https://discord.com/developers
zad adapter create discord \
    --application-id 1234567890 \
    --bot-token-env DISCORD_BOT_TOKEN \
    --scopes guilds,messages.send

# 2. Enable the adapter inside each project that should use it.
cd ~/code/my-project
zad adapter add discord
```

Use `--local` on `create` to store credentials only for the current
project (under `~/.zad/projects/<slug>/adapters/discord/`). Omit the
credential flags to run the interactive walkthrough instead.

## Usage

```
zad adapter <ACTION> <ADAPTER>
```

Actions today: `create` (register credentials) and `add` (enable for
this project). Today the only adapter is `discord`. See
[`man/main.md`](man/main.md) for the full reference — every command and
subcommand is in that single manpage.

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

**Missing `DISCORD_BOT_TOKEN`** — `zad adapter create discord` reads this
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

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under [MIT](LICENSE).