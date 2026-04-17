# Architecture of zad

A short tour of the codebase.

## Module layout

```
src/
  main.rs         — tokio entry: parses CLI, dispatches, returns exit code
  lib.rs          — crate root; re-exports the modules below
  error.rs        — `ZadError`, crate-wide `Result` alias
  logging.rs      — tracing subscriber + always-on rolling file appender
  cli/
    mod.rs        — clap root + `run()` dispatcher
    service.rs    — `zad service <action> <service>` group
    service_discord.rs — Discord handlers for `create` / `add`
  config/
    path.rs       — project-slug + `~/.zad/` path resolution
    schema.rs     — serde types: `ProjectConfig`, `ServiceRef`, `DiscordServiceCfg`
    mod.rs        — TOML read/write
  secrets/
    mod.rs        — keyring wrapper, with test-only in-memory backend
  service/
    mod.rs        — `Service` trait + domain types (Target, Message, Event, ManageCmd)
    discord/
      mod.rs      — `DiscordService` impl of `Service`
      client.rs   — thin wrapper around `serenity::http::Http`
      gateway.rs  — gateway listener → `BoxStream<Event>`
```

## Dependency direction

`cli` depends on `config`, `secrets`, and `service`. `service` depends on
`error`. `config` depends on `error`. `service::discord` is the only
module that links against serenity; every other module is transport-
agnostic. This keeps the `Service` trait reusable when more services are
added (Slack, GitHub, …).

## Command metadata

`clap` is the single source of truth for command names, usage, flag
specifications, defaults, and descriptions. The `--help-agent`,
`--debug-agent`, `zad commands`, `zad man`, and `zad docs` surfaces
mandated by `OSS_SPEC.md` §12 are not yet implemented project-wide;
when they are, they should introspect the clap command tree at runtime
(`Cli::command().get_subcommands()`, etc.) rather than duplicating the
metadata in a parallel registry.

## Config + secrets split

Per-project configuration lives at
`~/.zad/projects/<slug>/config.toml`; see `docs/configuration.md`.
Long-lived secrets (bot tokens, API keys) never land in the TOML — they
go to the OS keychain via the `secrets` module, keyed by
`service="zad"` and a service-specific account string.
