# Architecture of zad

A short tour of the codebase.

## Module layout

```
src/
  main.rs         — tokio entry: parses CLI, dispatches, returns exit code
  lib.rs          — crate root; re-exports the modules below
  error.rs        — `ZadError`, crate-wide `Result` alias
  logging.rs      — tracing subscriber + always-on rolling file appender
  output.rs       — structured human-readable printing helpers
  cli/
    mod.rs        — clap root + `run()` dispatcher
    lifecycle.rs  — `LifecycleService` trait + generic `run_{create,enable,disable,show,delete}<T>` driver shared by every service
    service.rs    — `zad service <action> <service>` group (clap enums + dispatch to the generic driver)
    service_list.rs    — `zad service list` rendering (shared across services)
    service_discord.rs — `DiscordLifecycle` impl of `LifecycleService`; Discord-specific prompts and token validation
    service_telegram.rs — `TelegramLifecycle` impl; Telegram-specific prompts and token validation
    discord.rs    — `zad discord <verb>` runtime handlers (send, read, channels, join, leave, discover, directory, permissions)
    telegram.rs   — `zad telegram <verb>` runtime handlers (send, read, chats, discover, directory, permissions)
    commands.rs   — `zad commands [NAME]... [--examples|--json]` — clap-tree introspection for the OSS_SPEC §12.4 discovery surface
    docs.rs       — `zad docs [TOPIC]` — prints `docs/*.md` embedded via `include_str!`
    man.rs        — `zad man [COMMAND]` — prints `man/*.md` embedded via `include_str!`
    help_agent.rs — renders the compact, prompt-injectable `--help-agent` text (§12.1)
    debug_agent.rs — renders the troubleshooting block for `--debug-agent` (§12.2)
  config/
    path.rs       — project-slug + `~/.zad/` path resolution
    schema.rs     — serde types: `ProjectConfig`, `ServiceProjectRef`, `DiscordServiceCfg`, `TelegramServiceCfg`
    directory.rs  — per-project `directory.toml` (name -> snowflake cache, Discord)
    mod.rs        — TOML read/write
  secrets/
    mod.rs        — keyring wrapper, with test-only in-memory backend
  permissions/
    mod.rs        — re-exports; shared primitives every service composes its policy from
    pattern.rs    — allow/deny lists (exact, glob, `re:<regex>`, numeric snowflake) evaluated against every alias of the target
    content.rs    — `deny_words` / `deny_patterns` / `max_length` for outbound bodies
    time.rs       — UTC allow-window (`days`, `windows`), supports cross-midnight ranges
  service/
    mod.rs        — `Service` trait + domain types (Target, Message, Event, ManageCmd)
                  + cross-service `DryRunOp` / `DryRunSink` / `StderrTracingSink`
    registry.rs   — `SERVICES: &[&str]` canonical list of services this build ships
    discord/
      mod.rs      — `DiscordService` impl of `Service`
      client.rs   — thin wrapper around `serenity::http::Http`
      transport.rs — `DiscordTransport` trait + live/dry-run impls for `--dry-run` preview
      gateway.rs  — gateway listener → `BoxStream<Event>`
      permissions.rs — Discord-specific `EffectivePermissions`; per-verb `check_<verb>_<target>` methods
    telegram/
      mod.rs      — `TelegramService` impl of `Service`
      client.rs   — reqwest wrapper over the Bot API (`getMe`, `sendMessage`, `getUpdates`, …)
      transport.rs — `TelegramTransport` trait + live/dry-run impls
      directory.rs — per-project `directory.toml` (name -> chat_id cache)
      permissions.rs — Telegram-specific `EffectivePermissions`; per-verb checks
```

## Dependency direction

`cli` depends on `config`, `secrets`, `permissions`, and `service`.
`service` depends on `error` and (via its `permissions.rs` submodules)
on the `permissions` primitives. `config` depends on `error`. Each
service's own module is the only module that links against that
provider's SDK — `service::discord` against `serenity`,
`service::telegram` against `reqwest` and the bare Bot API. Every
other module is transport-agnostic, which keeps the `Service` and
`LifecycleService` traits reusable when more services are added
(Slack, GitHub, …).

## Command metadata

`clap` is the single source of truth for command names, usage, flag
specifications, defaults, and descriptions. Every §12 discovery
surface introspects the same clap tree so they cannot drift from
`--help`:

- `--help-agent` (`src/cli/help_agent.rs`) — §12.1, compact
  prompt-injectable CLI description.
- `--debug-agent` (`src/cli/debug_agent.rs`) — §12.2, troubleshooting
  block with log paths, env vars, and diagnostic commands.
- `zad commands` (`src/cli/commands.rs`) — §12.4, command tree + flag
  reference + realistic examples + a machine-readable JSON dump
  consumed by the website extractor.
- `zad docs` / `zad man` (`src/cli/docs.rs`, `src/cli/man.rs`) — §12.3,
  conceptual topics and per-command reference pages embedded into the
  binary via `include_str!`.

## Config + secrets split

Per-project configuration lives at
`~/.zad/projects/<slug>/config.toml`; see `docs/configuration.md`.
Long-lived secrets (bot tokens, API keys) never land in the TOML — they
go to the OS keychain via the `secrets` module, keyed by
`service="zad"` and a service-specific account string.
