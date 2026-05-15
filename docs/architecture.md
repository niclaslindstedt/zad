# Architecture of zad

A short tour of the codebase.

## Workspace layout

`zad` is a Cargo workspace with two members. The dependency direction
is strictly one-way: the CLI depends on the library; the library never
imports anything from the CLI.

| Crate | Crate name | Binary / output | Role |
|---|---|---|---|
| `crates/zad` | `zad` | rlib | The library — services, config, secrets, OAuth, permissions, errors, logging. This is what Rust projects depend on directly via `zad = "0.6"`. |
| `crates/zad-cli` | `zad-cli` | `zad` binary | The CLI binary — clap parsing, interactive prompts, human/JSON output, dry-run echo machinery, embedded manpages and docs. Depends on the library by both path and version, so the published binary always pins a matching library release. |

`cargo install zad-cli` installs the same `zad` command users have
always run; `zad-cli` is the package name on crates.io, the binary it
produces is named `zad`.

## Module layout

```
crates/
  zad/
    src/
      lib.rs        — crate root; re-exports config/error/logging/
                      oauth/permissions/secrets/service modules
      error.rs      — `ZadError`, crate-wide `Result` alias
      logging.rs    — tracing subscriber + always-on rolling file appender
      config/
        mod.rs      — TOML read/write
        path.rs     — project-slug + `~/.zad/` path resolution
        schema.rs   — serde types: `ProjectConfig`, `ServiceProjectRef`,
                      `DiscordServiceCfg`, `GcalServiceCfg`,
                      `OnePassServiceCfg`, `SlackServiceCfg`,
                      `SpotifyServiceCfg`, `TelegramServiceCfg`,
                      `YmusicServiceCfg`
        directory.rs — per-project `directory.toml` shared schema
                      (Discord/Slack/Telegram name → ID caches)
      secrets/
        mod.rs      — keyring wrapper, with test-only in-memory backend
      oauth/
        mod.rs      — generic loopback OAuth flow (PKCE / non-PKCE,
                      HTTP and HTTPS-loopback) shared by every
                      OAuth-based service (gcal, spotify, ymusic)
      permissions/
        mod.rs      — re-exports; shared primitives every service
                      composes its policy from
        pattern.rs  — allow/deny lists (exact, glob, `re:<regex>`,
                      numeric snowflake) evaluated against every alias
                      of the target
        content.rs  — `deny_words` / `deny_patterns` / `max_length` for
                      outbound bodies
        time.rs     — UTC allow-window (`days`, `windows`), supports
                      cross-midnight ranges
        attachments.rs — file/MIME/size policy applied to outgoing
                      attachments
        service.rs  — generic `PermissionsService` trait powering the
                      shared staged-commit driver
        staging.rs  — staged-commit machinery (`.pending` files, diff,
                      discard, commit) used by every service's
                      `permissions <verb>` subcommands
        signing.rs  — Ed25519 signing keypair lookup + sign/verify for
                      permission file payloads
        trust.rs    — `~/.zad/signing/trusted.toml` per-machine trust
                      store: which permission files are authorized to
                      load, signed by the keychain key
        mutation.rs — typed mutations applied to a permissions TOML
                      AST (used by `permissions add` / `remove` /
                      `content` / `time`)
      service/
        mod.rs      — `Service` trait + domain types (`Target`,
                      `Message`, `Event`, `ManageCmd`)
                      + cross-service `DryRunOp` / `DryRunSink` /
                      `StderrTracingSink`
        registry.rs — `SERVICES: &[&str]` canonical list of services
                      this build ships
        lifecycle.rs — library-side lifecycle helpers (re-exported
                      types used by `LifecycleService` impls)
        discord/
          mod.rs    — `DiscordService` impl of `Service`
          client.rs — thin wrapper around `serenity::http::Http`
          transport.rs — `DiscordTransport` trait + live/dry-run impls
          gateway.rs — gateway listener → `BoxStream<Event>`
          permissions.rs — Discord-specific `EffectivePermissions`
          facade.rs — typed `Discord` library facade
                      (`SendRequest`, `ReadRequest`, `Discord::send`,
                      `Discord::read`, `Discord::with_paths`,
                      `Discord::from_default_config`)
        gcal/
          mod.rs    — `GcalHttp` client + domain types for Calendar v3
          client.rs — REST calls (calendarList, events.*, …) against
                      the minted access token
          transport.rs — `GcalTransport` trait + live/dry-run impls
          time.rs   — RFC 3339 / local-date parsing helpers
          permissions.rs — Gcal-specific `EffectivePermissions`;
                      per-verb checks + `[invite]` / `[remind]` blocks
          facade.rs — typed `Gcal` library facade
        onepass/
          mod.rs    — `OnePassService` wrapper over `op` child
                      processes
          client.rs — spawns `op` with `OP_SERVICE_ACCOUNT_TOKEN`
                      injected; parses JSON stdout
          permissions.rs — 1pass-specific filter-style permissions
                      (hidden-target semantics)
          facade.rs — typed `OnePass` library facade
        slack/
          mod.rs    — `SlackService` impl of `Service`
          client.rs — reqwest wrapper over the Web API
                      (chat.postMessage, conversations.*, users.*, …)
          transport.rs — `SlackTransport` trait + live/dry-run impls
          gateway.rs — Socket Mode listener → `BoxStream<Event>`
          permissions.rs — Slack-specific `EffectivePermissions`
          facade.rs — typed `Slack` library facade
        spotify/
          mod.rs    — `SpotifyService` wrapper + scope mapping
                      (`spotify_scopes_for`)
          client.rs — Spotify Web API v1 calls (search, playlists,
                      library) using the OAuth 2.0 PKCE refresh flow
          permissions.rs — Spotify-specific per-verb permissions
                      (search, playlists_read/write, library_read/write)
          facade.rs — typed `Spotify` library facade
        telegram/
          mod.rs    — `TelegramService` impl of `Service`
          client.rs — reqwest wrapper over the Bot API (`getMe`,
                      `sendMessage`, `getUpdates`, …)
          transport.rs — `TelegramTransport` trait + live/dry-run impls
          directory.rs — per-project `directory.toml`
                      (name → chat_id cache)
          permissions.rs — Telegram-specific `EffectivePermissions`
          facade.rs — typed `Telegram` library facade
        ymusic/
          mod.rs    — `YmusicService` wrapper + scope mapping
                      (`youtube_scopes_for`) and InnerTube constants
          client.rs — InnerTube (music.youtube.com/youtubei/v1) calls
                      for search, playlists, and library, with OAuth
                      access-token refresh against Google's TVHTML5
                      client
          oauth_device.rs — RFC 8628 device flow (the TVHTML5
                      client_id / client_secret are constants here)
          transport.rs — `YmusicTransport` trait + live/dry-run impls
          permissions.rs — YouTube-Music-specific per-verb permissions
          facade.rs — typed `Ymusic` library facade

  zad-cli/
    src/
      main.rs       — tokio entry: parses CLI, dispatches, returns exit
                      code
      lib.rs        — crate root; re-exports `cli` and `output`
      output.rs     — structured human-readable printing helpers
      cli/
        mod.rs      — clap root + `run()` dispatcher
        lifecycle.rs — `LifecycleService` trait + generic
                      `run_{create,enable,disable,show,delete}<T>`
                      driver shared by every service
        echo.rs     — dry-run echo machinery: when a permissions file
                      is unsigned, the would-be call is rendered to
                      stdout with exit 3 instead of issued (OSS_SPEC
                      §17.5)
        service.rs  — `zad service <action> <service>` group (clap
                      enums + dispatch to the generic driver)
        service_list.rs    — `zad service list` rendering
        service_status.rs  — `zad service status` aggregate (no
                      `--service` filter): pings every service in
                      parallel and emits one envelope for agents
        service_discord.rs  — `DiscordLifecycle`
        service_gcal.rs     — `GcalLifecycle` (OAuth 2.0 loopback +
                              `userinfo` validation; owns
                              `google_scopes_for`)
        service_onepass.rs  — `OnePassLifecycle` (1Password Service
                              Account token + `op whoami`)
        service_slack.rs    — `SlackLifecycle`
        service_spotify.rs  — `SpotifyLifecycle` (PKCE loopback)
        service_telegram.rs — `TelegramLifecycle`
        service_ymusic.rs   — `YmusicLifecycle` (OAuth 2.0 device
                              flow against Google's TVHTML5 client)
        onepass.rs    — `zad 1pass <verb>` runtime handlers
        discord.rs    — `zad discord <verb>` runtime handlers
        gcal.rs       — `zad gcal <verb>` runtime handlers
        slack.rs      — `zad slack <verb>` runtime handlers
        spotify.rs    — `zad spotify <verb>` runtime handlers
        telegram.rs   — `zad telegram <verb>` runtime handlers
        ymusic.rs     — `zad ymusic <verb>` runtime handlers
        signing.rs    — `zad signing <action>` — manage the local
                        Ed25519 key and the trust store
        permissions.rs — shared `zad <svc> permissions` staged-commit
                        driver (show, path, init, check, status, diff,
                        discard, commit, sign, add, remove, content,
                        time)
        commands.rs   — `zad commands [NAME]... [--examples|--json]`
                        — clap-tree introspection for the OSS_SPEC
                        §12.4 discovery surface
        docs.rs       — `zad docs [TOPIC]` — prints `docs/*.md`
                        embedded via `include_str!`
        man.rs        — `zad man [COMMAND]` — prints `man/*.md`
                        embedded via `include_str!`
        help_agent.rs — renders the compact, prompt-injectable
                        `--help-agent` text (§12.1)
        debug_agent.rs — renders the troubleshooting block for
                        `--debug-agent` (§12.2)
```

## Dependency direction

`zad-cli::cli` → `zad::service` + `zad::config` →
`zad::secrets` / `zad::oauth` / `zad::permissions`. Services never
import from `cli`; `config` never imports from `service`. `zad::error`
and `zad::logging` are leaf utilities imported by every layer.

Each service's own module is the only module that links against that
provider's SDK or transport — `service::discord` against `serenity`,
`service::slack`, `service::telegram`, `service::gcal`,
`service::spotify`, and `service::ymusic` against `reqwest` on top of
bare REST APIs, and `service::onepass` against the external `op` CLI
(spawned as a child process). Every other module is transport-agnostic,
which keeps the `Service` and `LifecycleService` traits reusable as
more services are added.

## Library facades (the typed entry points)

Each service exposes a typed library facade — the same code path the
CLI drives. For Discord the canonical shape is in
`crates/zad/src/service/discord/facade.rs`:

- A `Discord` struct constructed via `Discord::from_default_config()`
  (env-aware: honors `ZAD_HOME_OVERRIDE`, `ZAD_PERMISSIONS_PATH`,
  `ZAD_PERMISSIONS_ROOT`, `ZAD_SECRETS_MEMORY`) or
  `Discord::with_paths(...)` (env-free, deterministic — recommended
  for production library code, multi-tenant servers, and tests).
- One `*Request::new` constructor per verb that runs validation at
  construction time, so a `SendRequest` cannot be invalid by the time
  it reaches `Discord::send`.
- One `Discord::<verb>` async method per verb returning a typed
  `*Response`.

Every service follows the same shape. New services must ship both
constructors so library callers don't have to choose between
ergonomic-but-env-aware and deterministic-but-verbose entry points.

## Command metadata

`clap` is the single source of truth for command names, usage, flag
specifications, defaults, and descriptions. Every §12 discovery
surface introspects the same clap tree so they cannot drift from
`--help`:

- `--help-agent` (`crates/zad-cli/src/cli/help_agent.rs`) — §12.1,
  compact prompt-injectable CLI description.
- `--debug-agent` (`crates/zad-cli/src/cli/debug_agent.rs`) — §12.2,
  troubleshooting block with log paths, env vars, and diagnostic
  commands.
- `zad commands` (`crates/zad-cli/src/cli/commands.rs`) — §12.4,
  command tree + flag reference + realistic examples + a
  machine-readable JSON dump consumed by the website extractor.
- `zad docs` / `zad man` (`crates/zad-cli/src/cli/docs.rs`,
  `crates/zad-cli/src/cli/man.rs`) — §12.3, conceptual topics and
  per-command reference pages embedded into the binary via
  `include_str!`.

## Config + secrets split

Per-project configuration lives at
`~/.zad/projects/<slug>/config.toml`; see `docs/configuration.md`.
Long-lived secrets (bot tokens, API keys, OAuth refresh tokens) never
land in the TOML — they go to the OS keychain via the `secrets`
module, keyed by `service="zad"` and a service-specific account
string.
