# zad gcal

> Runtime verbs for the Google Calendar service — list calendars,
> read/create/update/delete events, invite attendees, and set
> reminders, all gated by a rich per-verb permissions policy.

## Synopsis

```
zad gcal <VERB> [OPTIONS]
```

## Description

`zad gcal` operates the Google Calendar service at runtime. The
project must already have `gcal` enabled (`zad service enable gcal`)
and valid OAuth credentials registered in either scope — runtime
verbs resolve the effective configuration with local winning over
global, load the three-field OAuth credential (client_id +
client_secret + refresh_token) from the OS keychain, and mint a
fresh access token once per CLI invocation.

| Verb | Description |
|---|---|
| `calendars list`        | List every calendar visible to the authenticated user. |
| `calendars show <id>`   | Show metadata for one calendar (or `primary`). |
| `events list`           | List events on a calendar, optionally filtered by time or free-text query. |
| `events show`           | Show one event by ID. |
| `events create`         | Create a new event. Supports all major fields and `--dry-run`. |
| `events update`         | Patch an existing event. `--add-attendee` / `--remove-attendee` / `--add-reminder-minutes` are additive/subtractive. |
| `events delete`         | Delete an event. |
| `permissions`           | Inspect, scaffold, or dry-run the per-project permissions policy. |
| `self`                  | Manage the `@me` alias that resolves to the authenticated email. |

Every verb supports `--json` for machine-readable output.

## Credentials (OAuth 2.0)

Google Calendar uses OAuth 2.0. `zad service create gcal` stores
three keychain entries per scope:

- `gcal-client-id:<scope>`
- `gcal-client-secret:<scope>`
- `gcal-refresh:<scope>`

Access tokens are **never persisted**: each CLI invocation exchanges
the refresh token for a fresh access token and uses it for the
lifetime of that process.

### Creating an OAuth client

1. Open the Google Cloud Console credentials page:
   `https://console.cloud.google.com/apis/credentials`.
2. Create an **OAuth client of type "Desktop app"**. Zero-page web
   clients and service accounts will *not* work with zad's loopback
   flow — you'll get a `redirect_uri_mismatch` on token exchange.
3. Enable the **Google Calendar API** under "APIs & Services →
   Library".
4. Run `zad service create gcal`. zad opens your browser to Google's
   consent screen, accepts the redirect on `http://127.0.0.1:<port>`,
   exchanges the authorization code for a refresh token with PKCE
   (S256), and stores all three keychain entries.

If you already have a refresh token (minted via Google's OAuth
Playground, for instance), pass `--refresh-token` (or
`--refresh-token-env`) to skip the browser flow — useful for CI.

## Calendar addressing

Google Calendar IDs are email-shaped strings:

- `primary` — the authenticated user's primary calendar.
- `alice@example.com` — another user's calendar you've been granted
  access to.
- `xxxxxxx@group.calendar.google.com` — a shared / group calendar.

Every runtime verb accepts any of these forms. Patterns in the
permissions file match against the raw input (an `@` prefix is
stripped for ergonomics) and the resolved ID, so `calendars.allow =
["primary"]` works naturally.

A `default_calendar` set in the service config (`zad gcal self set
--email` configures the sibling `self_email`) is used by any verb
that omits `--calendar`.

## Scope enforcement

Every runtime verb checks the required scope against the `scopes`
array in the effective credentials file **before** any network call.
Missing the scope returns a `scope denied` error that names the exact
file path to edit. The mapping is:

| Verb | Required scope |
|---|---|
| `calendars list`, `calendars show`   | `calendars.read` |
| `events list`, `events show`         | `events.read` |
| `events create`, `events update`, `events delete` | `events.write` |
| `events update --add-attendee`       | `events.invite` (policy block `[invite]`) |
| `events create --reminder-minutes`, `events update --add-reminder-minutes` | `events.remind` (policy block `[remind]`) |
| `permissions`, `self`                | none (local state only) |

Google-side OAuth scopes are computed from the zad scopes at create
time, so the consent screen shows the least possible surface. See
`google_scopes_for` in `src/cli/service_gcal.rs` for the mapping.

## Permissions (second layer)

Scope is the coarse gate — "is this family of operations enabled?".
**Permissions** are the fine gate — "is *this* call (to this
calendar, at this time, with this content, with these attendees,
this far in the future) allowed?". They live in an optional TOML
file at:

- Global: `~/.zad/services/gcal/permissions.toml`
- Local:  `~/.zad/projects/<slug>/services/gcal/permissions.toml`

Both files apply — a call must pass every file that exists. Missing
files contribute no restrictions. See
[`docs/configuration.md`](../docs/configuration.md) for the full
schema and [`examples/gcal-permissions/`](../examples/gcal-permissions/)
for a worked example.

### Per-verb blocks

| Block | Gates |
|---|---|
| `[list_calendars]`, `[get_calendar]` | read-side calendar access |
| `[list_events]`, `[get_event]`       | read-side event access |
| `[create_event]`                     | new events — allow-list calendars, attendees, body; numeric caps; send-updates; block shared calendars |
| `[update_event]`                     | same set, but for PATCH operations |
| `[delete_event]`                     | use default-deny + per-calendar opt-in here |
| `[invite]`                           | `--add-attendee` on create/update |
| `[remind]`                           | `--reminder-minutes` / `--add-reminder-minutes` |

### Numeric caps

Per-function numeric caps (intersect across global/local via `min()`):

- `max_future_days` — refuse events starting further than N days out.
- `min_notice_minutes` — refuse events starting in less than N
  minutes (the "agent creates a meeting five minutes from now" guard).
- `max_attendees` — total attendee count after the write is applied.
- `send_updates_allowed` — allow/deny against the literal strings
  `"none"`, `"external"`, `"all"`.
- `block_shared_calendars` — boolean; when true, writes are refused
  on any calendar whose `accessRole` isn't `"owner"`.

### Hard-coded safety cap

Regardless of the permissions file, reminder minutes are capped at
**40320** (four weeks) — zad fails early with a `PermissionDenied`
so the error points at the permissions file rather than at Google.

## Verbs

### `zad gcal calendars list`

List every calendar visible to the authenticated user, filtered by
the `[list_calendars]` policy.

```
zad gcal calendars list [--json]
```

### `zad gcal calendars show <ID>`

Show one calendar's metadata (summary, timezone, access role).

### `zad gcal events list --calendar <ID>`

List events on a calendar, optionally filtered by time range or
free-text query.

```
zad gcal events list --calendar primary \
    [--time-min <RFC3339>] [--time-max <RFC3339>] \
    [--query <STRING>] [--max <N>] [--json]
```

### `zad gcal events show --id <EVENT_ID>`

Show one event, including attendee list and HTML link.

### `zad gcal events create`

Create a new event. Every flag is optional on its own, but the verb
needs at least a summary and start/end to be useful.

```
zad gcal events create --calendar primary \
    --summary "Design review" \
    --start 2026-05-01T15:00:00-07:00 \
    --end 2026-05-01T16:00:00-07:00 \
    [--description <TEXT>] [--location <TEXT>] [--tz <IANA>] \
    [--attendee <EMAIL> ...] [--reminder-minutes <N> ...] \
    [--visibility default|public|private] \
    [--send-updates none|external|all] \
    [--recurrence "RRULE:FREQ=WEEKLY;COUNT=10" ...] \
    [--from-json <PATH|->] [--dry-run] [--json]
```

- `--start` / `--end` accept RFC3339 (`2026-05-01T15:00:00Z`),
  RFC3339 with offset (`...-07:00`), or a bare date (`2026-04-19`;
  creates an all-day event via Google's `start.date` field).
- `--tz` annotates the dateTime. If omitted, the calendar's default
  timezone is used server-side.
- `--attendee @me` is sugar for the email stored in `self_email`.
- `--from-json <PATH>` reads a full event payload from a file (or
  `-` for stdin); flag-derived fields layer on top.
- `--dry-run` prints the payload that would have been sent without
  hitting the network or the keychain — useful before credentials
  are even registered.

### `zad gcal events update --id <EVENT_ID>`

Patch an existing event. Any field flag overwrites that field.
Attendee/reminder edits are additive:

```
zad gcal events update --id abc123 --calendar primary \
    --add-attendee bob@mycompany.com \
    --remove-attendee alice@mycompany.com \
    --add-reminder-minutes 15 [--add-reminder-minutes 60] \
    [--summary ...] [--start ...] [...] [--dry-run] [--json]
```

`--add-attendee` and `--remove-attendee` are evaluated against the
current attendee list fetched from Google, then the merged list is
written back.

### `zad gcal events delete --id <EVENT_ID>`

Delete an event.

```
zad gcal events delete --id abc123 --calendar primary \
    [--send-updates none|external|all] [--dry-run] [--json]
```

Default-deny this verb in your permissions file if you want an agent
to be able to create events but not delete them.

### `zad gcal permissions {show|path|init|check}`

Standard permissions subgroup shared with every service. `check` lets
you dry-run any verb's policy without hitting the network:

```
zad gcal permissions check --function create_event \
    --calendar primary --attendee alice@mycompany.com \
    --body "deploy ok" --start 2026-05-01T15:00:00Z --attendee-count 3
```

Exits 0 on allow, 1 on deny (and prints the rule text + the file to
edit).

### `zad gcal self {show|set|clear}`

Manage the `self_email` field — the authenticated user's email that
resolves the literal `@me` in attendee targets. Set during `zad
service create gcal` automatically; override here if you're using a
shared credential for multiple inboxes.

## Error handling

- `invalid_grant` on refresh → "refresh token is no longer valid;
  re-run `zad service create gcal`".
- `rateLimitExceeded` / 429 → "Google Calendar rate-limited this
  client; back off before retrying".
- `redirect_uri_mismatch` at create time → your OAuth client is the
  wrong type in Google Cloud Console. It must be **Desktop app**,
  not Web application.

## See also

- `zad service create gcal` — register credentials.
- [`docs/configuration.md`](../docs/configuration.md) — full
  credentials + permissions schema.
- [`examples/gcal-permissions/`](../examples/gcal-permissions/) —
  worked example policy.
- [`docs/services.md`](../docs/services.md) — the cross-service
  model.
