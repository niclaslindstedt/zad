# Changelog

## [0.7.0] — Unreleased

**BREAKING.** The repository is now a Cargo workspace with two members:

- **`zad`** is now the **library crate** (typed Rust API) at
  `crates/zad/`. Depend on it from a Rust project with
  `zad = "0.7"` and call the per-service typed facades — no
  binary install, no shelling out.
- **`zad-cli`** is the **binary crate** at `crates/zad-cli/`. Install
  it with `cargo install zad-cli` (replaces `cargo install zad`). The
  produced executable is still named `zad`, so every existing CLI
  invocation (`zad service create discord`, `zad discord send …`)
  continues to work unchanged.

For Rust integrators, this is the headline change:

```toml
[dependencies]
zad = "0.7"
```

```rust
use zad::service::discord::{Discord, MessageBody, SendRequest};
use zad::service::{ChannelId, Target};

let discord = Discord::with_paths(
    bot_token, scopes, config_path,
    Some(&global_perms), Some(&local_perms),
)?;
let req = SendRequest::new(
    Target::Channel(ChannelId(123_456_789_012_345_678)),
    MessageBody::text("hi"),
    vec![],
)?;                              // validation runs here, typed error
let resp = discord.send(req).await?;
```

### Added

- Typed library facade for every service: `Discord`, `Slack`,
  `Telegram`, `Gcal`, `Spotify`, `Ymusic`, `OnePass`. Each ships
  with three constructors (`from_default_config` — CLI-equivalent,
  honors `ZAD_*` env vars; `with_token` / `with_credentials` —
  explicit, env-free; `with_paths` — fully explicit, env-free,
  recommended for production library code) and validating `*Request`
  types per verb (body length, attachment count, limit range,
  required-field non-emptiness all checked at construction).
- `permissions::*::load_from(global, local)` per service — env-free
  permission loading.
- `examples/discord-library/` — runnable example crate showing the
  pinned-dependency + typed-call pattern end-to-end.

### Changed

- `cargo install zad` → `cargo install zad-cli`. Pre-built binary
  installer (`scripts/install.sh`) is unchanged; the executable name
  is still `zad`.
- `Makefile`: every target uses `--workspace`; `make install` calls
  `cargo install --path crates/zad-cli`.
- `scripts/update-versions.sh` now rewrites the workspace's
  `[workspace.package].version` and the `zad-cli → zad` path-dep's
  pinned version in lockstep.
- The `From<dialoguer::Error>` impl for `ZadError` was removed; CLI
  call sites now go through the `DialoguerExt::into_zad()` helper.
- `cli::lifecycle` (and its `LifecycleService` trait + driver
  functions) lives in the CLI crate, not the library — it depends on
  `clap` and `dialoguer`.

### Migration

- **CLI users**: switch to `cargo install zad-cli`. The `zad` binary
  it installs is identical to before.
- **Anyone who used `use zad::cli::…` from another Rust crate**:
  those types now live under `zad_cli::cli::…` (the binary crate's
  library entry point). Or — much better — call the typed facade
  under `zad::service::…::<Service>` instead.

## [0.6.0]

- feat(ymusic): add YouTube Music service (#58)
