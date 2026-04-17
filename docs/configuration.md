# Configuration

zad stores per-project adapter configuration in a TOML file under the
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

## Discord adapter

Commands that drive it (documented in [`man/main.md`](../man/main.md)):

- `zad adapter create discord [--local]` — register credentials.
- `zad adapter enable discord` — enable the adapter in the current project.
- `zad adapter disable discord` — disable it again (leaves credentials intact).

Every command accepts `--json` for script-friendly structured output.

### Credentials file

Stored at **one** of:

- Global: `~/.zad/adapters/discord/config.toml`
- Local:  `~/.zad/projects/<slug>/adapters/discord/config.toml`

The project-local file wins over the global one for that project. The
format is flat (no `[adapter.discord]` wrapper — the path already
identifies the adapter):

```toml
application_id = "1234567890"
scopes         = ["guilds", "messages.read", "messages.send"]
default_guild  = "987654321"   # optional
```

| Key | Type | Default | Description |
|---|---|---|---|
| `application_id` | string | — | Discord application (bot) ID. Numeric snowflake. |
| `scopes` | `[string]` | `["guilds", "messages.read", "messages.send"]` | Capabilities the adapter is permitted to use. |
| `default_guild` | string? | — | Optional default guild (server) ID. |

### Project file

`~/.zad/projects/<slug>/config.toml` records which adapters are enabled
for the project. It never contains credentials.

```toml
[adapter.discord]
enabled = true
```

### Token storage

The bot token is stored in the OS keychain at:

- **service:** `zad`
- **account:** `discord-bot:global` (global creds) or `discord-bot:<slug>` (local creds).

Rotate a token by re-running `zad adapter create discord --force` (add
`--local` to target project-local credentials).

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
