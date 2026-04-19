# Services

A **service** in zad is an integration with an external system — a
chat platform, an issue tracker, a source-control host, anything a
long-lived bot identity can talk to over HTTP, WebSocket, or another
durable transport. Services are the unit zad ships, configures, and
permissions. An agent drives a service by its verbs (`zad <service>
<verb>`); a human administers it by its lifecycle commands (`zad
service <action> <service>`).

Today the shipped services are `discord`, `gcal` (Google Calendar),
and `telegram`. This document describes the shape every service
conforms to, so adding `slack`, `github`, or another provider is
mechanical rather than speculative.

## What a service is, operationally

A service is four things at once:

1. **A bot identity** at a third-party provider (Discord application,
   GitHub App installation, Slack app, …) with a long-lived
   credential — typically a bot token or OAuth refresh token.
2. **A credentials file** on disk that records everything *except*
   the secret: the application ID, the declared scopes, any
   non-secret defaults (e.g. a default guild). The secret itself
   lives in the OS keychain, never in the TOML.
3. **A Rust module** under `src/service/<name>/` that implements
   the `Service` trait (`src/service/mod.rs`) and translates between
   zad's domain types and the provider's SDK.
4. **A CLI surface** — a pair of command groups (`zad service …` for
   lifecycle, `zad <name> …` for runtime verbs) and a manpage at
   `man/<name>.md`.

Every service also exposes a uniform enablement state: a project opts
into a service by listing it in
`~/.zad/projects/<slug>/config.toml`. Registering credentials and
enabling the service are deliberately separate steps so one set of
credentials can back many projects.

## Anatomy of a service module

```
src/service/<name>/
  mod.rs          — struct implementing `Service` (send_message / read_messages / listen / manage)
  client.rs       — HTTP wrapper; translates domain types ↔ SDK types; enforces scopes locally
  transport.rs    — (optional) runtime-verb trait + live/dry-run impls for `--dry-run` preview
  gateway.rs      — (optional) event listener that produces a `BoxStream<Event>`
  permissions.rs  — per-service schema composed from the generic primitives under `src/permissions/`
```

The `Service` trait is intentionally small:

```rust
#[async_trait]
pub trait Service: Send + Sync {
    fn name(&self) -> &'static str;
    async fn send_message(&self, target: Target, body: &str) -> Result<MessageId>;
    async fn read_messages(&self, channel: ChannelId, limit: usize) -> Result<Vec<Message>>;
    async fn listen(&self) -> Result<BoxStream<'static, Event>>;
    async fn manage(&self, cmd: ManageCmd) -> Result<()>;
}
```

Shared domain types — `ChannelId`, `MessageId`, `UserId`, `Target`,
`Message`, `Event`, `ManageCmd` — live in `src/service/mod.rs` so
services never leak their SDK's types across the trait boundary. A
provider concept that has no zad equivalent (Discord threads, Slack
workspaces, GitHub repos) is exposed through service-specific methods
on the concrete client (e.g. `DiscordHttp::join_channel`), not the
trait.

## Three layers of access control

zad gates every service call through three independent layers. Each
is enforced *before* any network I/O.

| Layer | Question it answers | Where it lives |
|---|---|---|
| **Credentials** | Does zad even have a token for this service? | `~/.zad/services/<name>/config.toml` + OS keychain |
| **Scopes**      | Is this *family* of operations enabled? | `scopes = [...]` inside the credentials file |
| **Permissions** | Is *this specific call* (target, time, content) allowed? | Optional `permissions.toml` next to the credentials |

### Credentials

Credentials come in two scopes:

- **Global** at `~/.zad/services/<name>/config.toml` — shared across
  every project on the machine.
- **Local**  at `~/.zad/projects/<slug>/services/<name>/config.toml`
  — scoped to one project's working directory, where `<slug>` is
  the absolute path with `/`, `\`, and `:` replaced by `-`.

When both exist, the local file **replaces** the global one for that
project (credentials are *not* merged — write the full scope list each
time). Secrets are stored in the OS keychain at
`service="zad", account="<name>-<kind>:<scope>"` (for example
`discord-bot:global` or `discord-bot:-Users-alice-code-foo`).

### Scopes

The `scopes` array in the credentials file declares which families
of operations the service may perform. Every runtime verb names the
scope it requires; missing it fails with
`ZadError::ScopeDenied { service, scope, config_path }` — the error
message always names the exact file to edit.

Scopes are **coarse and declarative**. They don't depend on the
target, body, or time of day. They exist so a project can ship with
read-only Discord access while another project holds a separate set
of credentials that can also write.

### Permissions

Permissions narrow what a declared scope may actually touch. They
live in an optional TOML file next to the credentials:

- Global: `~/.zad/services/<name>/permissions.toml`
- Local:  `~/.zad/projects/<slug>/services/<name>/permissions.toml`

Unlike credentials, **both files apply simultaneously** — a call must
pass every file that exists, and a missing file contributes no
restrictions. This makes it safe to ship a strict global baseline: a
project can only add further restrictions, never loosen the rule.

Every service builds its schema on top of the same three primitives
in `src/permissions/`:

| Primitive | What it does | Typical keys |
|---|---|---|
| `pattern` | Allow/deny lists matched against a target alias. Supports exact names, `*`/`?` globs, and `re:<regex>`. | `channels.allow`, `channels.deny`, `users.allow`, `guilds.deny`, … |
| `content` | Deny-word and deny-regex screening for outbound bodies, plus an optional `max_length` cap measured in codepoints. | `deny_words`, `deny_patterns`, `max_length` |
| `time`    | UTC allow-window: which weekdays and which `HH:MM-HH:MM` slots admit calls (windows may cross midnight). | `days`, `windows` |

Each service declares **one block per runtime verb** (for Discord:
`[send]`, `[read]`, `[channels]`, `[join]`, `[leave]`, `[discover]`,
`[manage]`) and optional top-level `[content]` / `[time]` defaults
that every block inherits and can narrow further. Pattern lists run
against **every alias** of the target — the raw input (sigils
stripped), the resolved ID, and every directory entry that maps to
that ID — so a deny on `*admin*` fires even when the agent pastes
the raw snowflake.

Deny always beats allow; an empty allow list is "no positive
constraint", not "deny all". Violations surface as
`ZadError::PermissionDenied { function, reason, config_path }` —
same shape as the scope error, and again the message names the file
to edit.

## Dry-run preview (optional, per mutating verb)

Mutating runtime verbs may expose a `--dry-run` flag that short-circuits
the network call and prints what *would* have been sent. Dry-run is
intentionally **orthogonal** to the three access-control layers: scope
and permission checks still fire, so a preview respects the same policy
boundary as a live call. The keychain read is skipped, so `--dry-run`
works before a bot is even configured — a common agent workflow is
"preview the shape of my call, then register credentials and re-run
without the flag".

The interception layer is trait-based and reusable across services:

| Primitive | Lives in | Purpose |
|---|---|---|
| `DryRunOp`, `DryRunSink`, `StderrTracingSink` | `src/service/mod.rs` | Cross-service record + default sink (a summary via `tracing::info!` plus the JSON payload on stdout). Every service wrapper emits to the same sink type. |
| `<Name>Transport` | `src/service/<name>/transport.rs` | Service-specific trait over the runtime verbs the CLI layer calls. One method per verb, typed in zad's domain types (no SDK leakage). |
| Live impl for the HTTP client | same file | Blanket `impl <Name>Transport for <Name>Http` that delegates to the inherent methods — the live path is unchanged, the trait is a thin façade. |
| `DryRun<Name>Transport` | same file | Preview impl. Mutating verbs emit a `DryRunOp` and return a stub (`MessageId(0)` for sends, `Ok(())` for joins/leaves/channel creates). Read verbs return empty vectors — they're not dry-run-eligible by convention. |

The CLI factory that materialises a client (`discord_http_for` for
Discord) takes a `dry_run: bool`, runs the scope check unconditionally,
and then returns either `Box::new(<Name>Http::new(&token, …))` or
`Box::new(DryRun<Name>Transport::new(default_dry_run_sink()))` as a
`Box<dyn <Name>Transport>`. The per-verb handlers stay oblivious — they
call `transport.send(…)` and check `args.dry_run` only to suppress the
trailing `"Sent …"` line that would otherwise falsely claim success.

Convention: **`--dry-run` belongs only on mutating verbs** (writes
visible to the remote service). Reads have no side effect to preview,
and making them dry-run-capable forces the sink to invent data. Local
mutations (e.g. Discord's `discover` writing the directory cache) are
out of scope — dry-run is about external side effects, not local state.

## Standard file layout

```
~/.zad/
  services/<name>/
    config.toml             — global credentials for <name>
    permissions.toml        — global permissions policy for <name> (optional)
  projects/<slug>/
    config.toml             — records which services this project uses
    services/<name>/
      config.toml           — project-local credentials for <name> (optional)
      permissions.toml      — project-local permissions (optional)
      <service-specific>…   — e.g. discord's `directory.toml`
```

The project's own `config.toml` never contains credentials. It only
holds opt-in markers of the form:

```toml
[service.<name>]
enabled = true
```

A project is "using" a service iff that key is present.

## Standard CLI surface

Every service ships two command groups with identical shapes.

### Lifecycle — `zad service <action> <service>`

| Action | Meaning |
|---|---|
| `create <service>`  | Register credentials (global by default, `--local` for project-scoped). Prompts interactively or takes flags; validates the secret before storing it. |
| `enable <service>`  | Add `[service.<name>] enabled = true` to this project's config. Credentials must already exist in some scope. |
| `disable <service>` | Inverse of `enable`. Leaves credentials untouched. |
| `list`              | Table of every known service with credential scope + project enablement. |
| `show <service>`    | Effective configuration and both scopes' details (never prints the secret). |
| `delete <service>`  | Inverse of `create` — removes the config file at the chosen scope and clears the matching keychain entry. |

Every command accepts `--json` for machine-readable output. Every
command names the exact file path it read or wrote.

### Runtime — `zad <service> <verb>`

Runtime verbs are service-specific but follow a few conventions:

- The project must already be opted in (`zad service enable <service>`).
- Credentials are resolved with **local winning over global**.
- The required scope is checked locally before any network call.
- The required permission block is checked locally before any
  network call.
- Every verb supports `--json` and prints the exact file to edit on
  any denial.

Every service must also ship a `permissions` subgroup with four
verbs with identical names:

| Verb | Behaviour |
|---|---|
| `show` | Print both candidate file paths plus the body of whichever files exist. |
| `path` | Print the two candidate paths, one per line (script-friendly). |
| `init [--local] [--force]` | Write a starter policy with safe defaults. |
| `check --function <name> [--target <id\|name>] [--body <text>]` | Dry-run a proposed call without hitting the network; exits 0 on allow, 1 on deny, printing the reason and config path. |

The concrete flags on `check` depend on the service's target kinds —
for Discord, `--channel`, `--user`, and `--guild`.

## Name directories (optional per-service cache)

Any service that identifies resources by opaque IDs may ship a
project-local **directory** mapping ergonomic names to those IDs.
Discord stores one at
`~/.zad/projects/<slug>/services/discord/directory.toml`; a future
Slack service would store channel and user IDs the same way.

The directory is populated by a service-specific `discover` verb
(best-effort, re-runnable, merges on top of hand-authored entries),
inspected by `<service> directory`, and consumed implicitly whenever
a verb accepts `--channel`, `--user`, or `--guild` with a name
instead of a raw ID. Permission rules evaluate against every name
the directory knows for a resolved ID, so a deny pattern based on
names is robust even when the agent pastes a numeric ID.

## Adding a new service

Adding a service — Telegram, Slack, Reddit, GitHub App, Matrix, IRC,
whatever — goes through three points of contact: a registry entry, a
`LifecycleService` impl for the lifecycle commands, and (when the
service has runtime verbs) the usual `Service` trait + permissions +
manpage + example. The lifecycle surface is the same for every
service; the runtime surface is service-specific.

### 1. Pick a credential shape

Every service is one of these patterns. The shape drives which clap
helpers you flatten in and how many keychain entries you write.

| Shape | Example services | Helpers |
|---|---|---|
| One long-lived bot token | Discord, Telegram, Slack bot | `#[command(flatten)] BotTokenArgs` + one `secrets::account(NAME, "bot", scope)` entry |
| OAuth (client_secret + refresh_token) | Reddit, Google, Spotify | Declare your own `--client-id` / `--client-secret` / `--refresh-token` flags; store two keychain entries with `kind = "client-secret"` and `kind = "refresh"` |
| Keypair / PEM | GitHub App | A `--private-key-file` flag; store the PEM bytes under `kind = "pem"` (plus `app_id`/`installation_id` as non-secret `Cfg` fields) |
| User + password → access token | Matrix, IRC SASL | A `--username` flag + interactive password prompt; store just the derived access token under `kind = "access"` |

If your provider doesn't fit, pick the nearest shape and extend — the
trait doesn't care as long as `store_secrets` / `delete_secrets` /
`inspect_secrets` all agree on the list of accounts they touch.

### 2. Checklist

Items 1–8 give you a fully working `zad service {create, enable,
disable, show, delete} <name>`. Items 9–13 add runtime verbs (only
needed when the service actually *does* something; a lifecycle-only
service is a valid interim state).

1. **Register the service.** Add `"<name>"` to
   `SERVICES` in `src/service/registry.rs`.
2. **Create `src/service/<name>/mod.rs`.** At minimum: a struct you
   plan to hang runtime methods on. May be stubbed — the lifecycle
   commands don't need a working client.
3. **Extend `ProjectConfig`** in `src/config/schema.rs`: add
   `<name>(&self) -> Option<&ServiceProjectRef>`,
   `enable_<name>(&mut self)`, `disable_<name>(&mut self)`.
4. **Add the per-service config struct** in
   `src/config/schema.rs` (e.g. `TelegramServiceCfg { bot_username,
   scopes, default_chat_id }`) — serde-derived, flat keys. Non-secret
   fields only.
5. **Create `src/cli/service_<name>.rs`** — see the skeleton below.
6. **Add dispatch variants** to the five enums in
   `src/cli/service.rs` (`CreateService`, `EnableService`, …) and one
   match arm in each of the five match blocks, routing to
   `lifecycle::run_*::<<Name>Lifecycle>(a)`.
7. **Add `tests/cli_service_<name>_test.rs`** mirroring
   `tests/cli_service_discord_test.rs`.
8. **Run `make fmt lint build test`**. Lifecycle is now wired
   end-to-end. (`oss-spec validate .` is an on-demand conformance
   check — useful when introducing structural changes, not required
   for every PR.)

9. **Implement the `Service` trait** in `src/service/<name>/mod.rs`
   when you're ready to ship runtime verbs. Keep the provider's SDK a
   private dependency of the client — never leak its types across the
   trait boundary.
10. **Enforce scopes locally** in the client: each method asserts
    the scope it needs *before* the network call and returns
    `ZadError::ScopeDenied { service: NAME, scope, config_path }`
    otherwise.
11. **Compose a permissions schema** in `src/service/<name>/permissions.rs`
    from `PatternListRaw`, `ContentRulesRaw`, `TimeWindowRaw`, with
    one per-function block. Expose
    `EffectivePermissions { global, local }` with one
    `check_<verb>_<target>` method per runtime verb.
12. **Wire runtime verbs** under `src/cli/<name>.rs` (the group
    entrypoint used by `zad <name> <verb>`) and add
    `Command::<Name>(...)` to `src/cli/mod.rs`. Include the mandatory
    `permissions` subgroup with `show` / `path` / `init` / `check`.
13. **Optional: `--dry-run`** for mutating verbs via the
    `<Name>Transport` pattern described in the *Dry-run preview*
    section above. Reuse `default_dry_run_sink()` from
    `src/service/mod.rs`; don't reinvent the sink.
14. **Write `man/<name>.md`, ship `examples/<name>-permissions/`,
    and update `docs/configuration.md`** with the credentials
    schema and any service-specific files.

### 3. Paste-ready `LifecycleService` skeleton

Drop this into `src/cli/service_<name>.rs` and edit every line marked
`EDIT:`. Discord uses exactly this shape (see `src/cli/service_discord.rs`
for the canonical reference).

```rust
use async_trait::async_trait;
use clap::Args;

use crate::cli::lifecycle::{
    BotTokenArgs, CreateArgsBase, CreateArgsLike, LifecycleService, ScopesArg,
    SecretRef, resolve_bot_token, resolve_scopes,
};
use crate::config::{ProjectConfig, TelegramServiceCfg};  // EDIT: your Cfg
use crate::error::{Result, ZadError};
use crate::secrets::{self, Scope};

const DEFAULT_SCOPES: &[&str] = &["messages.read", "messages.send"];    // EDIT
const ALL_SCOPES: &[&str] = &["messages.read", "messages.send", "chats.manage"]; // EDIT

/// EDIT: secret material your service needs in the keychain.
/// One token for a bot; `{ client_secret, refresh_token }` for OAuth;
/// `{ pem: Vec<u8> }` for a GitHub App; etc.
pub struct TelegramSecrets {
    pub bot_token: String,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[command(flatten)] pub base: CreateArgsBase,
    #[command(flatten)] pub token: BotTokenArgs,      // EDIT: drop if not a bot-token service
    #[command(flatten)] pub scopes: ScopesArg,        // EDIT: drop if scopes don't apply
    /// EDIT: your service-specific non-secret fields.
    #[arg(long)] pub bot_username: Option<String>,
    #[arg(long)] pub default_chat_id: Option<String>,
}

impl CreateArgsLike for CreateArgs {
    fn base(&self) -> &CreateArgsBase { &self.base }
}

pub struct TelegramLifecycle;

#[async_trait]
impl LifecycleService for TelegramLifecycle {
    const NAME: &'static str = "telegram";                   // EDIT
    const DISPLAY: &'static str = "Telegram";                // EDIT
    type Cfg = TelegramServiceCfg;                           // EDIT
    type Secrets = TelegramSecrets;                          // EDIT
    type CreateArgs = CreateArgs;

    fn enable_in_project(cfg: &mut ProjectConfig) { cfg.enable_telegram(); }  // EDIT
    fn disable_in_project(cfg: &mut ProjectConfig) { cfg.disable_telegram(); } // EDIT

    async fn resolve(args: &CreateArgs, non_interactive: bool)
        -> Result<(TelegramServiceCfg, TelegramSecrets)>
    {
        // EDIT: prompt-or-fail for each Option<_> field in your args.
        // `resolve` is async so you can call provider APIs from here
        // (e.g. validate a user-supplied snowflake, poll for a
        // self-identity message) before returning the final Cfg.
        let bot_username = args.bot_username.clone()
            .ok_or(ZadError::MissingRequired("--bot-username"))?;
        let default_chat_id = args.default_chat_id.clone();
        let scopes = resolve_scopes(
            args.scopes.scopes.as_deref(), DEFAULT_SCOPES, ALL_SCOPES, non_interactive)?;
        let bot_token = resolve_bot_token(
            args.token.bot_token.as_deref(),
            args.token.bot_token_env.as_deref(),
            non_interactive, Self::DISPLAY)?;
        Ok((
            TelegramServiceCfg { bot_username, scopes, default_chat_id },
            TelegramSecrets { bot_token },
        ))
    }

    async fn validate(_cfg: &TelegramServiceCfg, s: &TelegramSecrets) -> Result<String> {
        // EDIT: call the provider's whoami/auth-test endpoint. On error:
        //   Err(ZadError::Service { name: Self::NAME, message: format!(...) })
        Ok(format!("<unvalidated: {} chars>", s.bot_token.len()))
    }

    fn store_secrets(s: &TelegramSecrets, scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let account = secrets::account(Self::NAME, "bot", scope);
        secrets::store(&account, &s.bot_token)?;
        Ok(vec![SecretRef { label: "token", account, present: true }])
        // For multi-secret services, write each piece and return one
        // SecretRef per keychain entry — the driver renders each as a
        // separate line in `show` / `create` output.
    }

    fn delete_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let account = secrets::account(Self::NAME, "bot", scope);
        secrets::delete(&account)?;
        Ok(vec![SecretRef { label: "token", account, present: false }])
    }

    fn inspect_secrets(scope: Scope<'_>) -> Result<Vec<SecretRef>> {
        let account = secrets::account(Self::NAME, "bot", scope);
        let present = secrets::load(&account)?.is_some();
        Ok(vec![SecretRef { label: "token", account, present }])
    }

    // Powers `zad service status <svc>` and `zad status` — the driver
    // reads secrets back out of the keychain and hands them to
    // `validate` as a live "does this credential work?" check.
    // Return `Ok(None)` if any required account is missing at the
    // given scope; the driver will report "credentials_present: false"
    // without surfacing an error.
    fn load_secrets(scope: Scope<'_>) -> Result<Option<TelegramSecrets>> {
        let account = secrets::account(Self::NAME, "bot", scope);
        Ok(secrets::load(&account)?.map(|bot_token| TelegramSecrets { bot_token }))
    }

    fn cfg_human(cfg: &TelegramServiceCfg) -> Vec<(&'static str, String)> {
        let mut out = vec![("bot", cfg.bot_username.clone())];               // EDIT
        if let Some(c) = &cfg.default_chat_id { out.push(("chat", c.clone())); }
        out
    }

    fn cfg_json(cfg: &TelegramServiceCfg) -> serde_json::Value {
        serde_json::json!({                                                  // EDIT
            "bot_username": cfg.bot_username,
            "default_chat_id": cfg.default_chat_id,
        })
    }

    fn scopes_of(cfg: &TelegramServiceCfg) -> &[String] { &cfg.scopes }

    // Optional: surface a URL the user should visit after `create`
    // succeeds (e.g. an install/authorize page). Default impl returns
    // None. When set, the URL is printed under the create banner and
    // also opened in the system browser unless `--no-browser` was
    // passed.
    // fn post_create_hint(cfg: &TelegramServiceCfg) -> Option<String> {
    //     Some(format!("https://t.me/{}", cfg.bot_username))
    // }
}
```

### 4. Optional: a `self` identity and `@me`

Services where "the user" is a meaningful address (Discord, Telegram,
Slack, …) can expose an optional `self_*_id` field on their `Cfg` so
`zad <svc> send --<target> @me` resolves to the caller's own
account. Discord's `self_user_id` and Telegram's `self_chat_id` are
the reference implementations:

- Add the field to the `Cfg` as `Option<…>` with `#[serde(default,
  skip_serializing_if = "Option::is_none")]`. It's non-secret config,
  not a keychain entry.
- Take an optional `--self-<thing>` flag on `CreateArgs`. In
  non-interactive mode use the flag verbatim; in interactive mode
  prompt (or run a provider-specific capture flow) after the token is
  resolved — `resolve` is async, so provider calls are fair game.
- In the runtime CLI, special-case the literal `@me` (case-insensitive)
  in the target resolver **before** the directory lookup. Emit a clear
  error when the self-ID isn't set, pointing at the `self` subcommand.
- Wire a `self {show,set,clear,…}` subcommand group that mirrors
  `permissions` — same four-verb shape — so the field is manageable
  after create.
- Permission patterns match the raw input alongside the resolved ID,
  so `deny = ["@me"]` works automatically. Document this in the
  service's `examples/*-permissions/README.md`.

Services where "self" has no useful meaning (Reddit bots, GitHub Apps,
etc.) simply skip this — the pattern is opt-in.

### 5. Golden rules

- **Secrets never go in the TOML.** `Cfg` fields are for non-secret
  material only; everything sensitive flows through `Secrets` and
  ends up in the OS keychain via `secrets::account(NAME, kind, scope)`.
- **Use the generic error.** Provider failures go through
  `ZadError::Service { name: NAME, message }`. Add bespoke variants
  only for *structured* failures whose callers need to match on them
  (e.g. `DiscordChannelNotFound`).
- **Keychain account strings are stable.** Once shipped, the
  `(NAME, kind)` pair in `secrets::account` is a user-visible
  identifier — renaming it orphans every existing stored token.
- **`Cfg` must round-trip through serde.** `config::save_flat` /
  `load_flat` write and read the flat form; no fields may rely on
  runtime state.
- **Don't touch other services.** The trait's associated types keep
  services independent — scopes, verbs, and flags for one service
  must not change when another is added.

The `Service` trait, the `LifecycleService` trait, the permission
primitives, and the credentials/scopes/permissions three-layer model
are the contract. Everything else — thread-like sub-resources,
gateway sessions, service-specific discovery — is fair game to
expose as service-specific methods and verbs.

## See also

- [`docs/configuration.md`](configuration.md) — credentials,
  permissions, and directory schemas in full.
- [`docs/architecture.md`](architecture.md) — module layout and
  dependency direction for the crate as a whole.
- [`man/service.md`](../man/service.md) — lifecycle command reference.
- [`man/discord.md`](../man/discord.md) — the reference
  implementation of a service's runtime verbs.
