# YouTube Music permissions policy — example

A worked example of a `permissions.toml` for the `ymusic` service.
Drop it at one of:

```
~/.zad/services/ymusic/permissions.toml                       # global
~/.zad/projects/<slug>/services/ymusic/permissions.toml       # local
```

If both files exist they **intersect** — every call must pass every
file that is present. A missing file contributes no restrictions, so
scope is the only gate when neither file exists.

## What the example shows

- Shared `[content]` defaults that deny common credential shapes and
  cap playlist titles / descriptions / search queries at 256
  characters.
- Shared `[time]` defaults that pin runtime to UTC business hours.
- `[playlists_write]` restricted to playlists whose title starts with
  `zad-` or `scratch-`, with deny rules that catch `*release*` and
  `*official*` as a safety net so the agent can't blindly rename or
  delete a curated playlist. Also tightens the time window further to
  `10:00-18:00`.
- `[library_write]` with a deny list of specific video IDs the
  operator wants to protect from like / unlike.

## Try it out

```sh
# Scaffold a project-local policy from this file.
cp examples/ymusic-permissions/permissions.toml \
   ~/.zad/projects/<slug>/services/ymusic/permissions.toml

# Inspect the effective policy.
zad ymusic permissions show

# Dry-run an action without hitting YouTube.
zad ymusic permissions check --function playlists_write --target zad-test
zad ymusic permissions check --function playlists_write --target official-mix
# (the second one fails — the deny list catches `*official*`)
```

## See also

[`docs/configuration.md`](../../docs/configuration.md) for the full
schema and [`man/ymusic.md`](../../man/ymusic.md) for the per-verb
reference.
