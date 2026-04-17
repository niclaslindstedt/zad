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

## Code of Conduct

By participating you agree to abide by the [Code of Conduct](CODE_OF_CONDUCT.md).

## Reporting security issues

See [SECURITY.md](SECURITY.md). Do **not** open public issues for security
problems.