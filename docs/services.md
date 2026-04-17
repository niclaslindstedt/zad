# Services

A **service** in zad is an integration with an external system — a
chat platform, an issue tracker, a source-control host, anything a
long-lived bot identity can talk to over HTTP, WebSocket, or another
durable transport. Services are the unit zad ships, configures, and
permissions. An agent drives a service by its verbs (`zad <service>
<verb>`); a human administers it by its lifecycle commands (`zad
service <action> <service>`).

Today the only shipped service is `discord`. This document describes
the shape every service conforms to, so adding `slack`, `github`, or
another provider is mechanical rather than speculative.

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
`ZadError::ScopeDenied { scope, config_path }` — the error message
always names the exact file to edit.

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

Checklist for a new provider:

1. **Create `src/service/<name>/`** with at least `mod.rs`,
   `client.rs`, and `permissions.rs`. Implement the `Service` trait
   in `mod.rs`. Keep the provider's SDK a private dependency of the
   client — never leak its types across the trait boundary.
2. **Enforce scopes locally** in the client. Each method asserts the
   scope it needs *before* the network call and returns
   `ZadError::ScopeDenied` otherwise.
3. **Compose a permissions schema** in `permissions.rs` from
   `PatternListRaw`, `ContentRulesRaw`, and `TimeWindowRaw`, with
   one per-function block. Expose
   `EffectivePermissions { global, local }` with one
   `check_<verb>_<target>` method per runtime verb.
4. **Wire the lifecycle commands** under `src/cli/service.rs`
   (add variants to the `Create* / Enable* / Disable* / Show*
   / Delete*` enums and dispatch to a new `service_<name>.rs`).
   Keep the CLI shape identical to the existing services.
5. **Wire the runtime verbs** under `src/cli/<name>.rs` (the group
   entrypoint used by `zad <name> <verb>`). Include the mandatory
   `permissions` subgroup with `show` / `path` / `init` / `check`.
6. **Optional: add `--dry-run` for mutating verbs.** Define a
   `<Name>Transport` trait in `src/service/<name>/transport.rs`,
   implement it for the live client, and ship a
   `DryRun<Name>Transport` that emits `DryRunOp` records to the shared
   sink. Have the client-factory return `Box<dyn <Name>Transport>` and
   switch on `args.dry_run`. Reuse `default_dry_run_sink()` from
   `src/service/mod.rs`; don't reinvent the sink.
7. **Write `man/<name>.md`** — one per-command manpage, kept in sync
   with the clap definitions.
8. **Ship `examples/<name>-permissions.toml`** and a runnable
   example under `examples/` demonstrating the happy path.
9. **Update `docs/configuration.md`** with the credentials schema,
   the scope list, and any service-specific files (directory
   caches, etc.).

The `Service` trait, the permission primitives, and the
credentials/scopes/permissions three-layer model are the contract.
Everything else — thread-like sub-resources, gateway sessions,
service-specific discovery — is fair game to expose as
service-specific methods and verbs.

## See also

- [`docs/configuration.md`](configuration.md) — credentials,
  permissions, and directory schemas in full.
- [`docs/architecture.md`](architecture.md) — module layout and
  dependency direction for the crate as a whole.
- [`man/service.md`](../man/service.md) — lifecycle command reference.
- [`man/discord.md`](../man/discord.md) — the reference
  implementation of a service's runtime verbs.
