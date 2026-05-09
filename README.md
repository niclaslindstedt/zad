# zad

A Rust library and CLI that connects AI agents to external services (Discord, Slack, Google Calendar, Spotify, Telegram, YouTube Music, 1Password) via scoped service configurations instead of MCP servers.

The project ships as two crates: **`zad`** (library — typed Rust API for embedding into other Rust projects) and **`zad-cli`** (binary — the `zad` command-line tool). The CLI is a thin wrapper over the library, so behaviour can't drift between the two.

[![CI](https://github.com/niclaslindstedt/zad/actions/workflows/ci.yml/badge.svg)](https://github.com/niclaslindstedt/zad/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![crates.io: zad](https://img.shields.io/crates/v/zad.svg?label=zad)](https://crates.io/crates/zad)
[![crates.io: zad-cli](https://img.shields.io/crates/v/zad-cli.svg?label=zad-cli)](https://crates.io/crates/zad-cli)

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

Pre-built binary (Linux & macOS, x86_64 or aarch64):

```sh
curl -fsSL https://raw.githubusercontent.com/niclaslindstedt/zad/main/scripts/install.sh | sh
```

The script downloads the latest GitHub release for your OS/arch and
installs into the first writable directory it finds on `$PATH`,
preferring `~/.local/bin`, then `~/bin`, then `/usr/local/bin`. It
prints where the binary landed. Override with `ZAD_INSTALL_DIR=/path`
or pin a tag with `ZAD_VERSION=v0.1.2`.

From source (requires Rust 1.88+):

```sh
cargo install zad-cli                 # crates.io
cargo install --path crates/zad-cli   # local checkout
```

`cargo install zad-cli` installs an executable named `zad`, so every
existing command (`zad service create discord`, `zad discord send …`)
keeps working unchanged.

### Use as a library (Rust)

If you're embedding zad into a Rust project, depend on the **library**
instead — no binary install required:

```toml
[dependencies]
zad = "0.6"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust
use zad::service::discord::{Discord, MessageBody, SendRequest};
use zad::service::{ChannelId, Target};

#[tokio::main]
async fn main() -> zad::Result<()> {
    let discord = Discord::from_default_config()?;
    let req = SendRequest::new(
        Target::Channel(ChannelId(123_456_789_012_345_678)),
        MessageBody::text("hi from a typed Rust call"),
        vec![],
    )?;
    let resp = discord.send(req).await?;
    println!("sent message {}", resp.message_id.0);
    Ok(())
}
```

`SendRequest::new` validates the body length, attachment count, and
empty-payload rule **at construction time**; wrong-shape calls surface
as `zad::ZadError::Invalid` before any network I/O. Newtypes
(`ChannelId`, `UserId`, `MessageId`) prevent passing the wrong kind of
snowflake at a call site. See [`examples/discord-library/`](examples/discord-library/)
for a complete runnable project.

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

# 5. (Optional) DM yourself. `zad service create discord` offers to
#    capture your user ID via Developer Mode; if skipped, set it later:
zad discord self set 1112223334445556
zad discord send --dm @me "reminder: file the time sheet"
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

Permission policies are signed with an Ed25519 keypair stored in your
OS keychain: `zad <svc> permissions init` generates the key on first
use and signs the starter template. Agents propose policy changes via
a staged workflow — mutations write to a `.pending` file, and only
`zad <svc> permissions commit` signs and replaces the live file:

```sh
# Agent:
zad discord permissions add --function send --target channel \
    --list deny --local 'deploy-*'

# You:
zad discord permissions diff --local
zad discord permissions commit --local   # prompts keychain
```

Load-time verification fails closed, so an agent with filesystem
access cannot silently widen a policy. See
[`docs/permissions.md`](docs/permissions.md) for the trust model and
failure modes.

## Usage

```
zad service <ACTION> <SERVICE>   # configuration (create / enable / list / …)
zad <SERVICE> <VERB>             # runtime operations (service-specific verbs)
```

Configuration actions: `create` (register credentials), `enable` /
`disable` (toggle for this project), `list`, `show`, `status` (ping
the provider to confirm credentials actually work), and `delete`.
`zad service status` without a `--service` filter runs `status`
across every service at once and is designed for agents — `--json`
emits a stable envelope and the exit code reflects whether every
configured service pinged successfully. Pass `--service <name>`
(e.g. `--service discord`) to narrow the check to one service.

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
- **`slack`**: `send`, `read`, `channels`, `discover`, `directory`,
  `permissions`, and `self`. `--channel` accepts a Slack ID (`C…`)
  or a name from the cache populated by `discover`; `--dm` accepts
  a user ID, name, or `@me` (resolved from the optional
  `self_user_id`). `send` takes `--dry-run` with the same semantics
  as Discord's. Library-side `Service::listen()` opens a Socket
  Mode connection when an App-Level Token (`xapp-...`) was supplied
  at create time.
- **`telegram`**: `send`, `read`, `chats`, `discover`, `directory`,
  and `permissions`. `--chat` accepts a signed `chat_id`
  (negative for groups/supergroups), a `@username` for public
  channels, or a directory alias seeded by `discover`. `send` takes
  `--dry-run` with the same semantics as Discord's.
- **`gcal`** (Google Calendar): `calendars list|show`,
  `events list|show|create|update|delete`, plus the usual
  `permissions` and `self` subgroups. OAuth 2.0 via an interactive
  browser loopback flow at `zad service create gcal` (PKCE + state;
  requires a Google Cloud "Desktop app" OAuth client). The
  permissions schema gates calendars, attendees, content, time
  windows, `max_future_days`, `min_notice_minutes`, `max_attendees`,
  `send_updates_allowed`, and `block_shared_calendars`. Mutating
  verbs support `--dry-run`.
- **`spotify`**: `search`, `playlists list|show|create|rename|
  delete|add|remove`, `library tracks|albums {list,save,unsave}`,
  and the usual `permissions` subgroup. OAuth 2.0 PKCE public
  client (no `client_secret`) via the same shared loopback flow as
  gcal — `zad service create spotify` only needs a Spotify Client
  ID. The permissions schema gates per-verb targets (playlist names
  / track / album URIs / search queries), content, and time
  windows.
- **`ymusic`** (YouTube Music): `search`, `playlists list|show|
  create|rename|delete|add|remove`, `library {list,like,unlike}`,
  and the usual `permissions` subgroup. Talks to the YouTube Data
  API v3 (YouTube Music shares the same surface — there is no
  separate API). Same Google OAuth Desktop-app shape as gcal.
  Mutating verbs support `--dry-run`. The permissions schema gates
  per-verb targets (playlist titles / IDs, video IDs, search
  queries), content, and time windows.
- **`1pass`** (1Password): `vaults`, `items`, `tags`, `get`, `read`,
  `inject`, `create`, `whoami`, and `permissions`. Wraps the official
  `op` CLI with the token stored in the OS keychain and injected as
  `OP_SERVICE_ACCOUNT_TOKEN` into spawned child processes — the token
  is never exported into the parent shell. Destructive `op` verbs
  (`item edit`, `item delete`, `vault create|edit|delete`, `user`,
  `group`, `events-api`, `run`) are intentionally not exposed. The
  filter-style permissions policy treats out-of-scope targets as if
  they don't exist (hidden-target semantics).

Every command takes `--json` for machine-readable output.

Today the shipped services are `1pass`, `discord`, `gcal`, `slack`,
`spotify`, `telegram`, and `ymusic`. See [`man/main.md`](man/main.md)
for the top-level overview and [`man/service.md`](man/service.md),
[`man/1pass.md`](man/1pass.md), [`man/discord.md`](man/discord.md),
[`man/gcal.md`](man/gcal.md), [`man/slack.md`](man/slack.md),
[`man/spotify.md`](man/spotify.md), [`man/telegram.md`](man/telegram.md),
and [`man/ymusic.md`](man/ymusic.md) for the full per-command
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
