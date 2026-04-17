# Configuration

zad stores per-project service configuration in a TOML file under the
user's home directory:

```
~/.zad/projects/<slug>/config.toml
```

`<slug>` is the absolute current working directory with every `/` (and
every `\` or `:` on Windows) replaced by `-` — the same convention Claude
Code uses for its per-project files. For example, working in
`/Users/alice/code/zad` yields the slug `-Users-alice-code-zad`.

Secrets (bot tokens, API keys) are **never** written to the TOML. They
live in the OS keychain under the `zad` service.

## Resolution

| Override | Effect |
|---|---|
| `ZAD_HOME_OVERRIDE` | Replaces `~/` when computing `~/.zad/`. Tests only. |
| `ZAD_SECRETS_MEMORY=1` | Swaps the OS keyring for a process-local in-memory store. Tests only. |

## Discord service

Commands that drive it (documented in [`man/main.md`](../man/main.md)):

- `zad service create discord [--local]` — register credentials.
- `zad service enable discord` — enable the service in the current project.
- `zad service disable discord` — disable it again (leaves credentials intact).

Every command accepts `--json` for script-friendly structured output.

### Credentials file

Stored at **one** of:

- Global: `~/.zad/services/discord/config.toml`
- Local:  `~/.zad/projects/<slug>/services/discord/config.toml`

The project-local file wins over the global one for that project. The
format is flat (no `[service.discord]` wrapper — the path already
identifies the service):

```toml
application_id = "1234567890"
scopes         = ["guilds", "messages.read", "messages.send"]
default_guild  = "987654321"   # optional
```

| Key | Type | Default | Description |
|---|---|---|---|
| `application_id` | string | — | Discord application (bot) ID. Numeric snowflake. |
| `scopes` | `[string]` | `["guilds", "messages.read", "messages.send"]` | Capabilities the service is permitted to use. |
| `default_guild` | string? | — | Optional default guild (server) ID. |

### Project file

`~/.zad/projects/<slug>/config.toml` records which services are enabled
for the project. It never contains credentials.

```toml
[service.discord]
enabled = true
```

### Token storage

The bot token is stored in the OS keychain at:

- **service:** `zad`
- **account:** `discord-bot:global` (global creds) or `discord-bot:<slug>` (local creds).

Rotate a token by re-running `zad service create discord --force` (add
`--local` to target project-local credentials).

### Directory (name -> snowflake)

`zad discord discover` walks the bot's visible guilds/channels/members
and writes a local directory file at:

```
~/.zad/projects/<slug>/services/discord/directory.toml
```

The file is plain TOML and is the canonical source for ergonomic names
used by `--channel`, `--dm`, and `--guild` on every runtime verb. It is
safe to hand-edit; `discover` upserts on top of existing entries rather
than overwriting the file.

```toml
generated_at_unix = 1713364920   # optional; set by `discover`

[guilds]
"main-server" = "999000000000000000"

[channels]
# "guild/channel" wins over "channel" when both exist and a guild
# context is known. A bare `general` still resolves when the caller
# doesn't pass a guild.
"main-server/general"   = "111000000000000000"
"main-server/announce"  = "112000000000000000"
"general"               = "111000000000000000"

[users]
"alice" = "1001000000000000000"
```

Manage it from the CLI:

- `zad discord directory` — list every entry.
- `zad discord directory set <kind> <name> <id>` — upsert, where
  `<kind>` is `guild`, `channel`, or `user`.
- `zad discord directory remove <kind> <name>` — idempotent delete.
- `zad discord directory clear --force` — wipe the file.

Member discovery uses the Discord `GET /guilds/{id}/members` endpoint,
which requires the **GUILD_MEMBERS** privileged intent to be enabled for
the bot in the developer portal. Without it, `discover` skips the
members phase and emits a one-line warning — it is explicitly
best-effort and never aborts the walk.

### Privileged intents

Reading message *content* from guild channels requires the
**MESSAGE_CONTENT** privileged intent to be enabled for the bot in the
Discord developer portal. Without it, the `body` field on gateway
`MessageCreated` events is empty for guild messages.

## Logging

zad always writes a rolling daily log file at a platform-appropriate
state directory (per `OSS_SPEC.md` §19.2):

| Platform | Path |
|---|---|
| Linux   | `~/.local/state/zad/debug.log` |
| macOS   | `~/Library/Application Support/zad/debug.log` |
| Windows | `%LOCALAPPDATA%\zad\debug.log` |

The global `--debug` flag additionally mirrors the log to stderr.
