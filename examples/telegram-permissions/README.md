# Telegram permissions policy — example

A worked example of a `permissions.toml` for the Telegram service.
Drop it at one of:

```
~/.zad/services/telegram/permissions.toml                       # global
~/.zad/projects/<slug>/services/telegram/permissions.toml       # local
```

If both files exist they **intersect** — every call must pass every
file that is present. A missing file contributes no restrictions, so
scope is the only gate when neither file exists.

## What the example shows

- Shared `[content]` defaults that deny obvious credential shapes and
  cap body length below Telegram's 4096-char hard limit.
- Shared `[time]` defaults that pin runtime to UTC business hours.
- Per-function blocks (`[send]`, `[read]`, `[chats]`, `[discover]`)
  that tighten the shared defaults further — notably an allow-list on
  `discover` so the walk doesn't index every chat the bot happens to
  see.

## Try it out

```sh
# Scaffold a project-local policy from this file.
cp examples/telegram-permissions/permissions.toml \
   ~/.zad/projects/<slug>/services/telegram/permissions.toml

# Inspect the effective policy.
zad telegram permissions show

# Dry-run an action without hitting the Bot API.
zad telegram permissions check --function send \
    --chat team-room --body "deploy ok"
```

Note: the `send` / `read` / `chats` / `discover` *runtime* verbs are
still stubbed while the Bot API integration is being written.
`permissions check` and `directory` are already live, so you can
author and validate a policy today.

See [`docs/configuration.md`](../../docs/configuration.md#permissions-file)
for the full schema.
