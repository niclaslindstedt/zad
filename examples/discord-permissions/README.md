# Discord permissions policy — example

A worked example of a `permissions.toml` for the Discord service. Drop
it at one of:

```
~/.zad/services/discord/permissions.toml                       # global
~/.zad/projects/<slug>/services/discord/permissions.toml       # local
```

If both files exist they **intersect** — every call must pass every
file that is present. A missing file contributes no restrictions, so
scope is the only gate when neither file exists.

## What the example shows

- Shared `[content]` defaults that deny obvious credential shapes and
  cap body length below Discord's 2000-char hard limit.
- Shared `[time]` defaults that pin runtime to UTC business hours.
- Per-function blocks (`[send]`, `[read]`, `[channels]`, …) that tighten
  the shared defaults further — notably a default-deny on
  `channels.manage` so the library layer cannot create or delete
  channels without an explicit opt-in.

## Try it out

```sh
# Scaffold a project-local policy from this file.
cp examples/discord-permissions/permissions.toml \
   ~/.zad/projects/<slug>/services/discord/permissions.toml

# Inspect the effective policy.
zad discord permissions show

# Dry-run an action without hitting Discord.
zad discord permissions check --function send \
    --channel general --body "deploy ok"
```

The same pattern applies to every future service: each provider
reuses the generic `content` / `time` / `allow` / `deny` primitives
under `src/permissions/` and names its own per-function blocks.

See [`docs/configuration.md`](../../docs/configuration.md#permissions-file)
for the full schema.
