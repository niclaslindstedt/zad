# Spotify permissions policy — example

A worked example of a `permissions.toml` for the `spotify` service.
Drop it at one of:

```
~/.zad/services/spotify/permissions.toml                       # global
~/.zad/projects/<slug>/services/spotify/permissions.toml       # local
```

If both files exist they **intersect** — every call must pass every
file that is present. A missing file contributes no restrictions, so
scope is the only gate when neither file exists.

## What the example shows

- Shared `[content]` defaults that deny common credential shapes and
  cap playlist names / descriptions / search queries at 256
  characters.
- Shared `[time]` defaults that pin runtime to UTC business hours.
- `[playlists_write]` restricted to playlists whose name starts with
  `zad-` or `scratch-`, with deny rules that catch `*release*` and
  `*official*` as a safety net so the agent can't blindly rename or
  delete a curated playlist. Also tightens the time window further
  to `10:00-18:00`.
- `[library_write]` with a deny list of specific track / album URIs
  the operator wants to protect from save / unsave.

## Try it out

```sh
# Scaffold a project-local policy from this file.
cp examples/spotify-permissions/permissions.toml \
   ~/.zad/projects/<slug>/services/spotify/permissions.toml

# Inspect the effective policy.
zad spotify permissions show

# Dry-run an action without hitting Spotify.
zad spotify permissions check --function playlists_write --target zad-test
zad spotify permissions check --function playlists_write --target official-mix
# (the second one fails — the deny list catches `*official*`)
```

## See also

[`docs/configuration.md`](../../docs/configuration.md) for the full
schema and [`man/spotify.md`](../../man/spotify.md) for the per-verb
reference.
