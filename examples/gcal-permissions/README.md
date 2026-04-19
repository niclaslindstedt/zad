# Google Calendar permissions policy — example

A worked example of a `permissions.toml` for the `gcal` service.
Drop it at one of:

```
~/.zad/services/gcal/permissions.toml                       # global
~/.zad/projects/<slug>/services/gcal/permissions.toml       # local
```

If both files exist they **intersect** — every call must pass every
file that is present. A missing file contributes no restrictions, so
scope is the only gate when neither file exists.

## What the example shows

- Shared `[content]` defaults that deny common credential shapes and
  cap event-description length at 5000 characters.
- Shared `[time]` defaults that pin runtime to UTC business hours.
- `[create_event]` hard-restricted to `primary`, with a 365-day
  future horizon, a 15-minute minimum notice, a 20-attendee cap, and
  `block_shared_calendars = true` so an agent can't create events
  on calendars the user doesn't own.
- Default-deny on `[delete_event]` so the agent can create but not
  delete events unless the operator opts individual calendars in.
- `[invite]` attendee allow-list that gates `--add-attendee` on
  `events update` independently of the base write permission — a
  caller can allow updates in general while still refusing unknown
  domains in the attendee list.

## Try it out

```sh
# Scaffold a project-local policy from this file.
cp examples/gcal-permissions/permissions.toml \
   ~/.zad/projects/<slug>/services/gcal/permissions.toml

# Inspect the effective policy.
zad gcal permissions show

# Dry-run an action without hitting Google.
zad gcal permissions check --function create_event \
    --calendar primary --attendee alice@mycompany.com \
    --body "deploy ok" --start 2026-05-01T15:00:00Z
```

## `@me`

Both the raw `@me` literal and the resolved `self_email` alias are
tested against the attendee allow/deny list, so:

```toml
[create_event]
attendees.allow = ["@me"]
```

accepts `--attendee @me` once `zad gcal self set --email <addr>` has
populated `self_email` in the service config.

## Hard-coded safety caps

Regardless of the permissions file, zad enforces:

- **Reminder minutes ≤ 40320** (four weeks) — Google's own upper
  bound, failed early with a zad-shaped `PermissionDenied` so the
  error points at the config rather than at the API response.

See [`docs/configuration.md`](../../docs/configuration.md) for the
full schema and [`man/gcal.md`](../../man/gcal.md) for the per-verb
reference.
