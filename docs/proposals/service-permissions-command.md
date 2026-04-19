# Proposal: `zad service permissions` (cross-service aggregate)

Status: draft
Author: agent research on branch `claude/service-permissions-research-7G61q`
Targets: `docs/services.md#standard-cli-surface`

## TL;DR

Add a new cross-service command group `zad service permissions …` that
aggregates over every entry in `src/service/registry.rs:SERVICES`, in
the same way `zad service list` and `zad status` already do. Per-service
`zad <service> permissions …` stays exactly as it is — the new group is
strictly additive and lets an agent answer "what are my policies?"
with one call instead of N.

## Why this is worth doing

Today the per-service permissions surface lives under each service:

- `zad discord permissions {show,path,init,check}` (`src/cli/discord.rs:1011–1257`)
- `zad telegram permissions {show,path,init,check}` (`src/cli/telegram.rs:743–977`)

Both follow the spec in `docs/services.md:238–249` and share the same
primitives (`src/permissions/{content,pattern,time}.rs`).

That asymmetry against other cross-service commands is the main pain
point. We already have two aggregates that walk the registry:

| Aggregate | What it does | File |
|---|---|---|
| `zad service list` | Table of cred/enable state per service | `src/cli/service_list.rs` |
| `zad status`       | Parallel provider ping across every service | `src/cli/status.rs` |

But for permissions there is no single-call answer to either of:

1. *"Which services currently have permission policies, and where do
   they live?"* — operator/audit question.
2. *"Before I run anything, what am I allowed to do?"* — pre-flight
   question that agents ask today by invoking two separate commands.

Bolt-on concerns that fall out of the same gap:

- **Init drift.** Each new service needs its own `permissions init`
  run. An opinionated bulk init would make onboarding a project
  symmetric with `zad service create` in "do the same thing for every
  service I have enabled."
- **Path enumeration for backup / audit / version control.** Users who
  keep their policy files in a dotfiles repo currently need to know
  the four paths per service; a single command that prints all
  candidate paths across every service is cheap to ship and useful.
- **Extension point.** When a third service lands (Slack, Matrix,
  Reddit…) the per-service surface will still be four verbs; the
  aggregate stays one command. That is the whole value proposition of
  `registry::SERVICES` — we should use it here too.

## Non-goals

- **Not** replacing `zad <service> permissions …`. Those stay: they
  are the authoritative per-service surface and the only place the
  check semantics are well-defined (different services have different
  target kinds — Discord has channels/users/guilds, Telegram has
  chats).
- **Not** a cross-service `check`. A body-level check could
  theoretically fan out, but a target-level check cannot — `--channel`
  is Discord-only, `--chat` is Telegram-only. Forcing a common shape
  here would regress the per-service UX. Operators who want "would
  anywhere admit this body?" can script `zad <svc> permissions check`
  in a loop; we don't need to ship that.
- **Not** a new storage location. Permission files stay where they
  are, in the per-service directories defined by
  `config::path::global_service_config_path` and
  `project_service_config_path_for`.

## Proposed surface

`zad service permissions <action>`:

| Action | Behaviour | Mirrors |
|---|---|---|
| `list`                     | Table: service × (global present, local present, effective). Same layout as `zad service list`. | `service list` |
| `show`                     | Per-service block: both paths, presence, summary of active rules (function names, deny-word count, time-window presence). Also dumps raw file bodies when asked with `--raw`. | `zad <svc> permissions show` (fanned out) |
| `path`                     | Every candidate path, one per line — 2 × N services in a stable order, script-friendly. | `zad <svc> permissions path` (fanned out) |
| `init [--local] [--force] [--services=a,b]` | Write starter templates across the registry; `--services` limits the set. | `zad <svc> permissions init` (fanned out) |

Every action accepts `--json`. JSON envelope mirrors `zad status`:

```json
{
  "command": "service.permissions.show",
  "services": [
    { "service": "discord",  "global": {...}, "local": {...}, "summary": {...} },
    { "service": "telegram", "global": {...}, "local": {...}, "summary": {...} }
  ]
}
```

`summary` stays per-service (the rule shapes differ) but uses a
common minimal schema: `{ functions: [..], content_deny_words: N,
has_time_window: bool, configured: bool }`. Anything richer stays
inside the per-service JSON by flattening the service's existing
show output — no schema invention required.

## Architecture

### New trait: `PermissionsService`

Add a thin trait alongside `LifecycleService` in
`src/cli/lifecycle.rs` (or a new `src/cli/permissions_lifecycle.rs`
if we want to keep that file focused). Each service plugs in roughly
four methods:

```rust
pub trait PermissionsService: LifecycleService {
    fn permissions_global_path() -> Result<PathBuf>;
    fn permissions_local_path()  -> Result<PathBuf>;
    fn permissions_starter() -> &'static str;
    fn permissions_summary() -> Result<PermissionsSummary>;
}
```

`DiscordLifecycle` and `TelegramLifecycle` each already own the
helpers this trait wraps (`discord::perms::global_path`,
`telegram::perms::global_path`, `starter_template`, `load_effective`).
The impls are one-liners.

### New dispatcher: `src/cli/service_permissions.rs`

Mirrors `src/cli/status.rs`:

- clap `Subcommand` enum `{List, Show, Path, Init}`
- One `run_*` per action
- Each walks `SERVICES` (or a subset for `--services`) and collects
  per-service rows using `tokio::join!` for symmetry with `status`
  (most of the work is sync file reads, but the trait allows async
  for a service whose policy summary involves a network call later)
- Human renderer: re-use the column-width helpers already in
  `service_list.rs` for `list`; for `show`, emit a banner per
  service and delegate to the per-service printer helpers.

### Wiring

Add one variant to `src/cli/service.rs::Action`:

```rust
Permissions(service_permissions::PermissionsArgs),
```

…with one new dispatch arm. Pattern matches the existing
`Action::List` and `Action::Status` arms exactly.

### Docs & manpages

- Extend `docs/services.md#standard-cli-surface` with a short
  "Aggregate permissions" subsection under "Lifecycle".
- New `man/service-permissions.md` (one page for all four actions).
- Update `man/service.md` subcommand table to include `permissions`.
- Update `README.md` quickstart snippet if it lists `zad service`
  subcommands.

### Tests

- `tests/service_permissions_cli_test.rs` (or similar) mirroring
  `tests/status_cli_test.rs`:
  - `list --json` in a temp home with zero, one, two services
    holding permission files
  - `show --json` shape stability
  - `path` emits exactly `2 * SERVICES.len()` lines
  - `init --services discord` writes only discord's file
  - Use the `common::contains_path` helper per `CLAUDE.md` test
    conventions when asserting substrings of paths.

## Implementation checklist

1. Trait: `PermissionsService` with four methods.
2. Impls: `DiscordLifecycle`, `TelegramLifecycle`.
3. Dispatcher module: `src/cli/service_permissions.rs` with
   `ServicePermissionsArgs` + four `run_*` functions.
4. Wire `Action::Permissions` in `src/cli/service.rs`.
5. Share a `PermissionsSummary` struct via `lifecycle`.
6. Manpage: `man/service-permissions.md`.
7. Update `man/service.md`, `docs/services.md`, README.
8. Tests under `tests/`.
9. Run `make fmt lint test` and the `update-manpages` / `update-docs`
   / `update-readme` skills to catch drift.

## Open questions

- **Init defaults:** should `zad service permissions init` (no
  `--services`) write templates for every registered service, or only
  those enabled in the current project? I lean "every registered
  service" to match `list` / `status` semantics, with a note in the
  manpage that `--services` narrows it.
- **Exit code for `show` when no files exist:** `zad status` exits 1
  on any failure; missing permission files are the unrestricted
  default, so `show` should probably always exit 0 regardless. Worth
  calling out in the manpage.
- **Cross-service `check` later?** Leaving out of v1 per non-goals,
  but if agents end up scripting `for svc in discord telegram; …`
  often, revisit with a very constrained shape
  (`--body` only, no target flags).
