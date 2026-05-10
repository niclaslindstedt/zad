# Agent guidance for zad

This file is the canonical source of truth for AI coding agents working in this
repo. `CLAUDE.md`, `.cursorrules`, `.windsurfrules`, `GEMINI.md`,
`.aider.conf.md`, and `.github/copilot-instructions.md` are symlinks to this
file.

## OSS Spec conformance

This repository adheres to [`OSS_SPEC.md`](OSS_SPEC.md), a prescriptive
specification for open source project layout, documentation, automation, and
governance. A copy of the spec lives at the repository root so contributors and
AI agents can consult it without leaving the repo; its version is recorded in
the YAML front matter at the top of the file.

When in doubt about a layout, naming, or workflow decision, consult the
relevant section of `OSS_SPEC.md` — it is the source of truth for the
conventions this repo follows. `oss-spec validate .` is available as an
on-demand check; it is not part of the routine dev loop.

## Build and test commands

```sh
make build         # developer build
make test          # full test suite
make lint          # zero-warning linter
make fmt           # format in place
make fmt-check     # verify formatting (CI)
```

## Commit and PR conventions

- All commits follow [Conventional Commits](https://www.conventionalcommits.org/).
- PRs are squash-merged; the **PR title** becomes the single commit on `main`,
  so it must follow conventional-commit format.
- Breaking changes use `<type>!:` or a `BREAKING CHANGE:` footer.

## Architecture summary

`zad` is a Cargo workspace with two members:

- **`crates/zad`** — the library. Holds all business logic: services
  (`service::*`), TOML schema and path helpers (`config`), OS-keychain
  I/O (`secrets`), OAuth flows (`oauth`), permissions
  (`permissions`), errors (`error`), logging (`logging`). This is what
  Rust projects depend on directly via `zad = "0.6"`.
- **`crates/zad-cli`** — the CLI binary. Holds argument parsing
  (clap), interactive prompts (`dialoguer`), human/JSON output
  formatting, the dry-run echo machinery, embedded manpages and docs.
  Produces an executable named `zad` (so `cargo install zad-cli`
  installs the same `zad` command users have always run).
  Depends on the library by both path and version, so the binary
  always pins a matching library release.

Dependency direction is strictly layered: `zad-cli::cli` →
`zad::service` + `zad::config` → `zad::secrets`. Services never import
from `cli`; `config` never imports from `service`. `zad::error` and
`zad::logging` are leaf utilities imported by all layers.

Each service exposes a typed library facade — e.g. `Discord` with
`SendRequest::new`, `ReadRequest::new`, `Discord::send`,
`Discord::read`. Validation runs at request construction; library and
CLI go through the same code path so behavior can't drift between
them. See `crates/zad/src/service/discord/facade.rs` for the canonical
pattern.

The lifecycle commands (`zad service {create,enable,disable,show,delete}
<name>`) are driven by a single generic driver in
`crates/zad-cli/src/cli/lifecycle.rs`: each service implements the
`LifecycleService` trait in its own
`crates/zad-cli/src/cli/service_<name>.rs` (~80 lines) and the driver
handles the plumbing. The canonical list of services ships in
`crates/zad/src/service/registry.rs`. See
`docs/services.md#adding-a-new-service` for the full recipe — adding a
service (Telegram, Slack, Reddit, GitHub App, Matrix, IRC, …) is a
checklist-driven task, not a copy-paste of Discord.

### Env vars and the library

The library exposes both env-aware and env-free constructors per
service: `Discord::from_default_config()` honors `ZAD_HOME_OVERRIDE`,
`ZAD_PERMISSIONS_PATH`, `ZAD_PERMISSIONS_ROOT`, and
`ZAD_SECRETS_MEMORY` (CLI-equivalent shortcut), while
`Discord::with_paths(...)` reads zero env vars and is the recommended
entry point for production library code, multi-tenant servers, and
deterministic tests. New services should ship both shapes.

## Where new code goes

| Change type | Goes in |
|---|---|
| Library logic | `crates/zad/src/...` |
| New service (library side) | `crates/zad/src/service/<name>/` — `client.rs`, `permissions.rs`, `transport.rs`, `facade.rs` (typed `*Request`/`*Response` + a `<Service>` struct with `from_default_config` / `with_token` / `with_paths`). Add a row to `crates/zad/src/service/registry.rs`. |
| New service (CLI side) | `crates/zad-cli/src/cli/<name>.rs` (clap dispatch + render) and `crates/zad-cli/src/cli/service_<name>.rs` (`LifecycleService`). Wire into `crates/zad-cli/src/cli/service.rs`. |
| Library tests | `crates/zad/tests/...` |
| CLI tests | `crates/zad-cli/tests/...` |
| Docs update | `docs/...` |
| Examples | `examples/...` (TOML permission examples; `examples/<svc>-library/` for runnable Rust crates) |
| LLM prompt | `prompts/<name>/<major>_<minor>.md` (see `prompts/README.md`) |

## Test conventions

- **All tests live in separate files** — never inline in source files (no `#[cfg(test)]` blocks, no `if __name__ == "__main__"` test harnesses). This keeps source files free of test scaffolding and lets agents, hooks, and linters treat source and test code differently.
- Test files are named with a `_test` or `_tests` suffix (e.g. `check_test.rs`, `utils_test.py`). The stem must match the pattern `_?[Tt]ests?$` per §20 of `OSS_SPEC.md`.
- Library tests live in `crates/zad/tests/`; CLI integration tests (anything that drives the binary via `assert_cmd`) live in `crates/zad-cli/tests/`. Use `tempfile` or equivalent for any test that writes to the filesystem.
- **Any assertion that substrings a filesystem path out of stdout/stderr MUST use `common::contains_path("…/…")` instead of raw `predicates::str::contains`.** Windows CI renders paths with `\` separators and a plain `contains("a/b/c")` silently fails there even though it passes on Unix. The helper exists in both `crates/zad/tests/common/mod.rs` and `crates/zad-cli/tests/common/mod.rs` (kept in sync); write the author-facing fragment with forward slashes.

## Documentation sync points

When you change… | Update…
--- | ---
library public API | `docs/`, `README.md` "Use as a library" section, `examples/<svc>-library/` |
CLI flags  | `man/<cmd>.md`, `README.md` Quick start |
config keys| `docs/configuration.md` |
service facade | typed-input contract test (`crates/zad/tests/<svc>_facade_test.rs`) |

## Parity / cross-cutting rules

- Every new service (`src/service/<name>/`) must have a matching manpage at
  `man/<name>.md` and at least one runnable example under `examples/`.
- Each top-level `zad` command gets its own `man/<command>.md`
  (`main.md`, `service.md`, `discord.md`, …); `main.md` is the thin
  overview and the rest are per-command references. Keep them in sync
  with every clap subcommand and flag defined in `src/cli/` whenever
  commands, flags, or subcommands are added, removed, or renamed.

## Permissions pattern (cross-service)

Every service with runtime side effects layers **permissions** on top
of scope. Scope answers "is this family of operations enabled?";
permissions answer "is *this* call — to this target, at this time, with
this content — allowed?". The generic primitives live under
`src/permissions/` (`pattern`, `content`, `time`) and are specialized
per service under `src/service/<name>/permissions.rs`.

Non-negotiable semantics shared by every service:

- Two-scope files: `~/.zad/services/<svc>/permissions.toml` (global)
  and `~/.zad/projects/<slug>/services/<svc>/permissions.toml` (local).
  Credentials replace across scopes; **permissions intersect** — both
  files apply, strictest wins, a missing file contributes nothing.
  Local can only tighten global, never loosen it.
- Top-level `[content]` and `[time]` defaults; one block per runtime
  verb, each optionally narrowing those defaults. Deny always beats
  allow; an empty allow list is "no positive constraint" (not "deny
  all"). Patterns support exact names, globs (`*`, `?`), numeric
  snowflakes, and `re:<regex>` for full regex.
- Rules run against **every alias** of the target: the raw input
  (sigils stripped), the resolved ID as a string, and every directory
  entry that maps to that ID — so a deny on `*admin*` fires even when
  the agent pastes the raw snowflake.
- Enforcement happens **before any network call**, in the CLI layer
  where the directory is in scope.
- Error shape is `ZadError::PermissionDenied { function, reason,
  config_path }`; every message names the file to edit.
- Every service exposes four `permissions` subcommands with the same
  names: `show`, `path`, `init [--local] [--force]`, and `check
  --function <name> [--channel|--user|--guild <id|name>] [--body <text>]`.

When adding a new service, build its schema on top of the generic
primitives (`PatternListRaw`, `ContentRulesRaw`, `TimeWindowRaw`),
expose `EffectivePermissions { global, local }` with one
`check_<verb>_<target>` method per runtime verb, wire the four
`permissions` subcommands, and ship a realistic example at
`examples/<service>-permissions/` (subdirectory with `permissions.toml`
and a `README.md`, per §13 of `OSS_SPEC.md`).

## Website staleness policy

Per §11.2 of `OSS_SPEC.md`, the website must be regenerated whenever
commands, configuration keys, or examples change. Run `make website` locally
(or push to `main` — the `pages` CI workflow rebuilds and deploys
automatically). If `make website` is not available yet, the `pages` CI job
will catch drift on every push to `main`.

## Maintenance skills

Per §21 of `OSS_SPEC.md`, this repo ships agent skills for keeping drift-prone artifacts in sync with their sources of truth. Skills live under `.agent/skills/<name>/` and are also accessible via the `.claude/skills` symlink.

| Skill | When to run |
|---|---|
| `maintenance`     | When several artifacts have likely drifted at once — umbrella skill that runs every sync skill in the correct order. |
| `update-manpages` | After any change to the clap CLI tree (subcommand added / renamed, flag added or removed, default changed). |
| `update-docs`     | After any change to the public API, configuration keys, or error messages. |
| `update-readme`   | After any change that alters user-visible behavior, commands, or install instructions. |
| `update-website`  | After README, `docs/`, `OSS_SPEC.md` front matter, or `Cargo.toml` version moves — the website extractor reads from those. |
| `sync-oss-spec`   | After `OSS_SPEC.md` itself moves, or as the final pass of a drift sweep to catch residual structural violations the per-artifact skills did not touch. |

Each skill has a `SKILL.md` (the playbook) and a `.last-updated` file (the baseline commit hash). Run a skill by loading its `SKILL.md` and following the discovery process and update checklist. The skill rewrites `.last-updated` at the end of a successful run, and improves itself in place when it discovers new mapping entries. The `maintenance` skill owns a **Registry** table listing every `update-*` skill — add a row whenever you create a new sync skill. Skills are accessible via the `.claude/skills` symlink (→ `.agent/skills/`).