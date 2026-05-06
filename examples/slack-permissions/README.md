# slack-permissions example

A realistic Slack permissions policy for a workplace agent. Copy it to the
global or project-local scope, review the allow/deny patterns, and tighten
as needed for your workspace.

## Quick start

```sh
# Initialize the global policy from the built-in starter template:
zad slack permissions init

# Or copy this example manually:
cp examples/slack-permissions/permissions.toml \
   ~/.zad/services/slack/permissions.toml

# Sign it (required before zad will enforce it):
# The init command signs automatically. If you copied manually, run:
zad slack permissions init --force   # re-generates and signs
```

## What this policy does

| Section | Effect |
|---|---|
| `[content]` | Blocks common credential patterns and limits body length |
| `[time]` | Restricts calls to UTC business hours (Mon–Fri 08:00–20:00) |
| `[send].channels.allow` | Bot may only post to `general`, `bot-*`, `zad-*`, `team-*` channels |
| `[send].channels.deny` | Always blocks channels matching `*admin*`, `*ops*`, `*incident*`, `*security*` |
| `[read].channels.deny` | Bot may not read `*private*` or `*confidential*` channels |

## Tightening the policy

**Restrict to specific users:**
```toml
[send]
users.allow = ["alice", "bob", "carol"]
```

**Allow only one channel by exact name:**
```toml
[send]
channels.allow = ["zad-bot"]
channels.deny  = ["*"]
```

**Add a regex deny pattern to block phone numbers:**
```toml
[content]
deny_patterns = ["(?i)bearer\\s+[a-z0-9._-]+", "\\b\\d{3}[-.\\s]\\d{4}\\b"]
```

**Allow calls at any time (remove time window):**
```toml
[time]
# No days or windows = no time restriction.
```

## Checking the policy

```sh
# Check whether a send to #general is allowed:
zad slack permissions check --function send --channel general

# Check with a body:
zad slack permissions check --function send --channel general \
    --body "the api_key is here"

# Show both policy files and their paths:
zad slack permissions show

# Print paths only (script-friendly):
zad slack permissions path
```

## The `deny always beats allow` rule

An empty `allow` list means "no positive constraint" — any target is
admitted unless it matches a `deny`. Add entries to `allow` to create a
whitelist; the bot is then restricted to exactly those targets.

```toml
# This allows only general and team-*:
channels.allow = ["general", "team-*"]
channels.deny  = []

# This denies admin-* even if general is in allow:
channels.allow = ["general"]
channels.deny  = ["*admin*"]
```

## Pattern syntax

- Exact name: `"general"` — matches only `#general`
- Glob: `"bot-*"` — matches any channel starting with `bot-`
- Question mark glob: `"team-?"` — matches `team-a`, `team-b`, etc.
- Regex: `"re:^(general|random)$"` — full regex anchored to the value

Permission patterns run against **every alias** of the target — the
input as typed (sigils stripped), the resolved channel ID, and any
directory entries mapping to that ID. So `deny = ["*admin*"]` fires even
when the agent pastes the raw `C...` ID of an admin channel.
