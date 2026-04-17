# Contributing to zad

Thanks for your interest! This document describes how to set up a dev
environment, the conventions we follow, and how to get a change merged.

## Prerequisites

- **Rust 1.88+** (edition 2024) — install via [rustup](https://rustup.rs/). Run
  `rustup update stable` to get the latest stable toolchain.
- **cargo** — ships with Rust; no separate install needed.
- **clippy** and **rustfmt** — `rustup component add clippy rustfmt`.
- **make** — used to run the project's convenience targets (`make build`,
  `make test`, etc.). Comes pre-installed on macOS and most Linux distros.
- An OS keychain zad can write to: macOS Keychain, Linux Secret Service
  (gnome-keyring or KWallet), or Windows Credential Manager.

## Getting the source

```sh
git clone https://github.com/niclaslindstedt/zad.git
cd zad
```

## Build, test, lint

```sh
make build
make test
make lint
make fmt-check
```

## Development workflow

1. Fork the repo.
2. Create a topic branch: `git checkout -b feat/<slug>` or `fix/<slug>`.
3. Make focused commits using [Conventional Commits](https://www.conventionalcommits.org/):
   ```
   <type>(<scope>): <summary>
   ```
   Types: `feat`, `fix`, `perf`, `docs`, `test`, `refactor`, `chore`, `ci`,
   `build`, `style`. Breaking changes: `<type>!:` or `BREAKING CHANGE:` footer.
4. Open a PR. The **PR title** must be conventional-commit format because we
   squash-merge and that title becomes the commit message on `main`.
5. CI must be green and at least one reviewer must approve.

## Tests

Tests live in `tests/` as standalone Rust integration-test files. File names
must end with `_test` or `_tests` (e.g. `service_test.rs`). There are no
inline `#[cfg(test)]` blocks in source files.

Run the full suite:
```sh
make test
# or directly:
cargo test
```

Run a single test by name:
```sh
cargo test <test_name>
```

No formal coverage target is enforced, but every public function that contains
non-trivial logic should have at least one test.

## Documentation

If your change touches user-visible behavior, update the relevant `docs/`
topic and the README quick start. See `AGENTS.md` for the full sync table.

## Pre-commit hooks

A [`.pre-commit-config.yaml`](.pre-commit-config.yaml) ships with the
repo. Install the hooks once after cloning:

```sh
pipx install pre-commit   # or: pip install --user pre-commit
pre-commit install
pre-commit install --hook-type commit-msg
```

Hooks mirror the CI gate (`fmt-check`, `lint`, trailing whitespace,
conventional-commit message format). Running `git commit` will now
refuse to commit anything that would fail CI.

## Governance

`zad` follows a **BDFL** governance model while it is pre-1.0:

- **Merge rights.** [@niclaslindstedt](https://github.com/niclaslindstedt)
  is the only maintainer with commit and merge rights on `main`.
  All other contributors land changes via pull request.
- **Decisions.** Design decisions are made in issues, RFC-style PRs, or
  discussion threads. The maintainer has final say on merges and
  release cadence, but welcomes disagreement in the thread — the goal
  is the best outcome for `zad`, not maintainer ego.
- **Adding maintainers.** When the project outgrows a single
  maintainer, additional committers will be invited on the basis of
  sustained, high-quality contributions. The maintainer team will
  promote governance from BDFL to a **maintainer team** model and this
  section will be updated accordingly.
- **Disagreements.** When a contributor and the maintainer cannot
  agree, the maintainer's decision stands, but the reasoning must be
  documented in the PR or issue so it can be revisited later.
- **Fork / transfer.** This is an open source project; the `zad`
  codebase is MIT-licensed and anyone may fork it. If the maintainer
  steps away for an extended period, a well-formed fork under a new
  maintainer is welcomed and this repository will be archived with a
  pointer to the successor.

## Where to discuss

- **Bugs and feature requests** — open a [GitHub issue](https://github.com/niclaslindstedt/zad/issues).
- **Questions, ideas, show-and-tell** — use [GitHub Discussions](https://github.com/niclaslindstedt/zad/discussions).
- **Security reports** — the private channel in [SECURITY.md](SECURITY.md).

## Code of Conduct

By participating you agree to abide by the [Code of Conduct](CODE_OF_CONDUCT.md).

## Reporting security issues

See [SECURITY.md](SECURITY.md). Do **not** open public issues for security
problems.