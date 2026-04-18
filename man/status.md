# zad status

> Aggregate live-status check of every configured service in one call.

## Synopsis

```
zad status [--json]
```

## Description

`zad status` pings every service registered in zad's internal service
registry and reports whether each one's credentials actually work. It
is the agent-facing counterpart to `zad service status <svc>`: a single
call covers every service, with one JSON envelope the agent can consume
to decide which services are usable in the current environment.

Per service, the command:

1. Loads the global and local config files (if any).
2. Determines the *effective* scope (local wins over global).
3. Reads the secret for the effective scope out of the OS keychain.
4. Calls the provider's lightweight identity endpoint (Discord's
   `GET /users/@me`, Telegram's `getMe`). The identity the provider
   returns is reported as `authenticated_as`.

Only the effective scope is pinged — pinging both `global` and `local`
when both are configured would double the per-run provider rate-limit
cost. The non-effective scope is still reported (`configured`,
`credentials_present`) so an agent can see what's on disk without
spending a second API call.

The provider calls run in parallel across services, so adding services
doesn't linearly inflate the command's latency.

## Flags

| Flag | Type | Default | Description |
|---|---|---|---|
| `--json` | bool | `false` | Emit machine-readable JSON instead of human-readable text. Recommended when an agent is the consumer. |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Every configured service's effective scope pinged successfully. Services with no credentials at all (`effective: null`) do **not** count as failures — "not configured" is different from "broken". |
| 1 | At least one configured service's effective scope failed (auth rejected, network error, missing keychain entry). |
| 2 | Usage error. |

## JSON shape

```json
{
  "command": "status",
  "ok": true,
  "services": [
    {
      "service": "discord",
      "effective": "global",
      "ok": true,
      "global": {
        "path": "...",
        "configured": true,
        "credentials_present": true,
        "check": { "ok": true, "authenticated_as": "mybot" }
      },
      "local":  { "path": "...", "configured": false, "credentials_present": false },
      "project": { "config": "...", "enabled": true }
    },
    {
      "service": "telegram",
      "ok": false,
      "global": { "path": "...", "configured": true, "credentials_present": false },
      "local":  { "path": "...", "configured": false, "credentials_present": false },
      "project": { "config": "...", "enabled": false }
    }
  ]
}
```

Each entry in `services` has the same shape as the envelope emitted
by `zad service status <svc>`, minus the top-level `command` field
(that's moved out to the aggregate).

`effective` is omitted from a service row when the service isn't
configured at all. `check` appears only on the effective scope.

## Examples

```sh
# Human-readable summary, one row per service
zad status

# Agent use: JSON + exit code
if zad status --json > /tmp/zad-status.json; then
  echo "all good"
else
  jq '.services[] | select(.ok == false)' < /tmp/zad-status.json
fi

# Pluck just the working services
zad status --json | jq '.services[] | select(.ok) | .service'
```

## See also

- [`zad man service`](service.md) — per-service `status`, `show`,
  `create`, `enable`, `disable`, `delete`.
- [`zad man main`](main.md) — top-level CLI overview.
