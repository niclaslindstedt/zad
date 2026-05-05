# zad slack

> Runtime verbs for the Slack service — send, read, list channels,
> discover and curate a name → ID directory.

## Synopsis

```
zad slack <VERB> [OPTIONS]
```

## Description

`zad slack` operates the Slack service at runtime. The project must
already have Slack enabled (`zad service enable slack`) and valid
credentials registered in either scope — runtime commands resolve the
effective configuration with local winning over global, then load the
matching bot token from the OS keychain.

| Verb | Description |
|---|---|
| `send` | Send a message to a channel or a direct message to a user. |
| `read` | Fetch recent messages from a channel. |
| `channels` | List channels visible to the bot in the workspace. |
| `discover` | Walk the workspace's channels and members and cache a name → ID map. |
| `directory` | Inspect or hand-edit that cache. |
| `permissions` | Inspect, scaffold, or dry-run the per-project permissions policy. |
| `self` | Manage the Slack user ID resolved from the literal `@me` in `--dm` targets. |

Every verb supports `--json` to emit machine-readable output instead
of the human-readable default.

## Scope enforcement

Every runtime verb checks the required scope against the `scopes` array
in the effective credentials file **before** any network call. Missing
the scope returns a `scope denied` error that names the exact file path
to edit. The mapping is:

| Verb | Required scope |
|---|---|
| `send` (channel) | `chat:write` |
| `send` (DM) | `im:write` + `chat:write` |
| `read` (public channel) | `channels:history` |
| `read` (DM channel) | `im:history` |
| `channels` | `channels:read` |
| `discover` (channels) | `channels:read` |
| `discover` (members) | `users:read` |
| `directory` | none (local state only) |

See `docs/configuration.md` for the full scope list and for the
local-vs-global precedence rules.

## Permissions (second layer)

Scope is the coarse gate — "is this family of operations enabled?".
**Permissions** are the fine gate — "is *this* call (to this target, at
this time, with this content) allowed?". They live in an optional
TOML file at:

- Global: `~/.zad/services/slack/permissions.toml`
- Local:  `~/.zad/projects/<slug>/services/slack/permissions.toml`

Both files apply simultaneously (strictest wins). A missing file
contributes no restrictions. Initialize a starter policy with:

```
zad slack permissions init
```

### Permission functions

| Function | Checked for |
|---|---|
| `send` | `--channel` or `--dm` target, message body |
| `read` | `--channel` target |
| `channels` | workspace-level listing |
| `discover` | workspace-level walk |

## Slack IDs

Slack identifies resources with string IDs:

| Prefix | Resource |
|---|---|
| `C...` | Public channel |
| `D...` | DM / IM channel |
| `G...` | Group DM |
| `U...` | User |
| `T...` | Workspace (team) |

Commands that accept `--channel` and `--dm` resolve inputs in this
order:

1. If the value looks like a Slack ID (starts with `C`, `D`, `G`, `U`
   etc. and is ≥ 8 alphanumeric characters), use it verbatim.
2. Look the value up in the project's `directory.toml` (after stripping
   any leading `#` or `@`).
3. Fail with a clear error pointing at `zad slack discover` or
   `zad slack directory set`.

## Directory

```
~/.zad/projects/<slug>/services/slack/directory.toml
```

The directory maps human-friendly names to Slack IDs. It is populated
by `zad slack discover` and can be hand-edited via `zad slack directory`.
Channels are keyed by bare name (e.g. `general`). Users are keyed by
display name and by username.

## Socket Mode (`listen`)

The `Service::listen()` method uses Slack **Socket Mode** for real-time
events. This requires an App-Level Token (`xapp-...`) in addition to the
bot token. Without it, `listen()` returns an empty stream.

To enable Socket Mode:

1. In your Slack app dashboard → Socket Mode → Enable.
2. Create an App-Level Token with `connections:write` scope.
3. Supply it via `--app-token` when running `zad service create slack`,
   or update it later with `zad service create slack --local --app-token <token>`.

## Examples

```sh
# Send a message
zad slack send --channel C1234567890 "Hello from zad"

# Send by name (after discover)
zad slack send --channel general "Hello from zad"

# DM yourself
zad slack send --dm @me "reminder: review PRs"

# Read recent messages
zad slack read --channel general --limit 10

# List channels
zad slack channels

# Discover workspace layout
zad slack discover

# Check permissions before sending
zad slack permissions check --function send --channel general

# Preview a send without hitting Slack
zad slack send --channel C1234567890 --dry-run "test body"
```

## See also

- `man/service.md` — lifecycle commands (`create`, `enable`, `disable`, `show`, `delete`).
- `docs/configuration.md` — credentials, scopes, and permissions schema.
- `examples/slack-permissions/` — realistic starter permissions policy.
