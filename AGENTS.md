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

Run `oss-spec validate .` to verify conformance. When in doubt about a layout,
naming, or workflow decision, consult the relevant section of `OSS_SPEC.md` —
it is the source of truth for the conventions this repo follows.

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

`zad` is a single Rust crate whose entry point is `src/main.rs`. The crate
is split into four top-level modules: `cli` (argument parsing via clap),
`service` (one sub-module per integration, e.g. `service::discord`),
`config` (TOML schema and path helpers), and `secrets` (OS-keychain I/O).
`src/lib.rs` re-exports the public surface so that integration tests under
`tests/` can reach it without going through the binary.

Dependency direction is strictly layered: `cli` → `service` + `config` →
`secrets`. Services never import from `cli`; `config` never imports from
`service`. `src/error.rs` and `src/logging.rs` are leaf utilities imported
by all layers. Adding a new service means adding a sub-module under
`src/service/`, implementing the shared service trait defined in
`src/service/mod.rs`, and wiring a new subcommand in `src/cli/`.

## Where new code goes

| Change type | Goes in |
|---|---|
| New feature | `src/...` |
| Tests       | `tests/...` |
| Docs update | `docs/...` |
| Examples    | `examples/...` |
| LLM prompt  | `prompts/<name>/<major>_<minor>.md` (see `prompts/README.md`) |

## Test conventions

- **All tests live in separate files** — never inline in source files (no `#[cfg(test)]` blocks, no `if __name__ == "__main__"` test harnesses). This keeps source files free of test scaffolding and lets agents, hooks, and linters treat source and test code differently.
- Test files are named with a `_test` or `_tests` suffix (e.g. `check_test.rs`, `utils_test.py`). The stem must match the pattern `_?[Tt]ests?$` per §20 of `OSS_SPEC.md`.
- Tests live in `tests/`. Use `tempfile` or equivalent for any test that writes to the filesystem.

## Documentation sync points

When you change… | Update…
--- | ---
public API | `docs/`, `README.md` Quick start
CLI flags  | `man/<cmd>.md`, `README.md`
config keys| `docs/configuration.md`

## Parity / cross-cutting rules

- Every new service (`src/service/<name>/`) must have a matching manpage at
  `man/<name>.md` and at least one runnable example under `examples/`.
- The `man/main.md` command reference must stay in sync with every clap
  subcommand defined in `src/cli/`. Update it whenever commands, flags, or
  subcommands are added, removed, or renamed.

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
| `maintenance`   | When several artifacts have likely drifted at once — umbrella skill that runs every `update-*` skill in the correct order. |
| `update-docs`   | After any change to the public API, configuration keys, or error messages. |
| `update-readme` | After any change that alters user-visible behavior, commands, or install instructions. |

Each skill has a `SKILL.md` (the playbook) and a `.last-updated` file (the baseline commit hash). Run a skill by loading its `SKILL.md` and following the discovery process and update checklist. The skill rewrites `.last-updated` at the end of a successful run, and improves itself in place when it discovers new mapping entries. The `maintenance` skill owns a **Registry** table listing every `update-*` skill — add a row whenever you create a new sync skill.