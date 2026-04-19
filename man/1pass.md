# zad 1pass

> Runtime verbs for the 1Password service — list vaults, items, and
> tags; fetch item metadata; resolve `op://` secret references;
> inject secrets into templates; create new items, all gated by a
> permissions policy that treats out-of-scope targets as if they
> don't exist.

## Synopsis

```
zad 1pass <VERB> [OPTIONS]
```

## Description

`zad 1pass` wraps the official
[1Password CLI](https://developer.1password.com/docs/cli/) (`op`),
exposing only the agent-safe subset of its verbs. The project must
already have the `1pass` service enabled (`zad service enable 1pass`)
and a Service Account token registered in either scope. Runtime verbs
resolve the effective configuration with local winning over global,
load the token from the OS keychain, and spawn `op` child processes
with `OP_SERVICE_ACCOUNT_TOKEN` injected into their environment —
the token is never exported into the parent shell.

Intentionally **not** exposed: `op item edit`, `op item delete`,
`op item share`, `op document`, `op user`, `op group`,
`op vault create|edit|delete`, `op events-api`, `op run`. These
surfaces can destroy vault state or exfiltrate audit data and have no
agent-friendly framing.

| Verb | Description |
|---|---|
| `vaults`      | List the vaults visible to this account (filtered by policy). |
| `items`       | List items, optionally filtered by vault/tag/category (filtered by policy). |
| `tags`        | Enumerate distinct tags across visible items. |
| `get`         | Fetch item metadata; denied fields are stripped, not errored on. |
| `read`        | Resolve a single `op://vault/item/field` reference. |
| `inject`      | Substitute every `op://…` reference in a template. Each ref is gated individually before `op` sees the template. |
| `create`      | Create a new item in an explicitly-allowed vault. |
| `whoami`      | Diagnostic — confirm the stored token works. |
| `permissions` | Inspect, scaffold, or dry-run the permissions policy. |

Every verb supports `--json` for machine-readable output.

## Authentication (Service Account)

`zad 1pass` authenticates **only** via a
[1Password Service Account](https://developer.1password.com/docs/service-accounts/).
Desktop-app biometric unlock is disabled (`OP_BIOMETRIC_UNLOCK_ENABLED=false`)
so an interactive prompt can never surface behind an agent. To
register credentials:

```sh
zad service create 1pass \
    --account my.1password.com \
    --token-env OP_SERVICE_ACCOUNT_TOKEN \
    --scopes read,write
```

`--account` is the sign-in URL of your 1Password tenant (commonly
`my.1password.com`, `team.1password.eu`, or similar). The token is
stored in the OS keychain under the account
`1pass-service-account:{global|<slug>}`. Rotation is admin-side —
revoke the old token in the 1Password console and re-run
`zad service create 1pass --force`.

## Scope enforcement

Two zad-level scopes gate the runtime verbs:

| Scope | Verbs |
|---|---|
| `read`  | `vaults`, `items`, `tags`, `get`, `read`, `inject`, `whoami` |
| `write` | `create` |

Missing the scope returns a `scope denied` error naming the exact
file path to edit.

## Hidden-target semantics (the security story)

Every read-side verb runs the permissions filter **before** returning
`op`'s output to the caller. Anything your policy doesn't let the
agent see is presented as if it doesn't exist:

- `vaults` and `items` silently drop filtered rows.
- `get` on a hidden item returns `"foo" isn't an item in any vault
  visible to this account` — the same shape `op` emits when an item
  genuinely doesn't exist.
- `read` and `inject` do the same for hidden `op://` references.
- `get` on a visible item with hidden fields returns the item with
  those fields stripped from the `fields` array — the agent can't
  even learn that a `recovery_code` field exists.

`create` is the single exception: writes always surface
`PermissionDenied` naming the rule, so an agent learns *why* its
write was refused (which is information the operator chose to
reveal by configuring `[create]` at all).

## Permissions policy

Files live at:

- global: `~/.zad/services/1pass/permissions.toml`
- local:  `~/.zad/projects/<slug>/services/1pass/permissions.toml`

Both files intersect — local can only *tighten* global, never
loosen. The schema composes the five target axes — vaults, tags,
items, categories, fields — plus `[content]`, `[time]`, and a
special deny-by-default `[create]` block.

Top-level defaults apply to every read-side verb unless a
per-verb block overrides them:

```toml
# read-side defaults, applied to vaults/items/tags/get/read/inject
vaults.allow     = []
vaults.deny      = []
tags.allow       = []
tags.deny        = []
items.allow      = []
items.deny       = []
categories.allow = []
categories.deny  = []
fields.allow     = []
fields.deny      = []

[content]
deny_words    = ["password", "api_key"]
deny_patterns = []
max_length    = 50000

[time]
days    = ["mon", "tue", "wed", "thu", "fri"]
windows = ["09:00-18:00"]

# Per-verb narrowing. Any axis you write here replaces the top-level
# default for this verb. Leave blocks empty to inherit.
[vaults]        # narrows `zad 1pass vaults`
[items]         # narrows `zad 1pass items`
[tags]          # narrows `zad 1pass tags`
[get]           # narrows `zad 1pass get`
[read]          # narrows `zad 1pass read`
[inject]        # narrows `zad 1pass inject`

[create]
# DENY-BY-DEFAULT: if `[create].vaults.allow` is absent OR empty,
# every create call is rejected.
[create.vaults]
allow = ["AgentWork"]
[create.categories]
allow = ["Login", "API Credential", "Secure Note"]
[create.tags]
allow = ["agent-managed"]   # every created item must carry this tag
```

## Permission subcommands

| Subcommand | Purpose |
|---|---|
| `show`                 | Print the two candidate file paths and their bodies. |
| `path`                 | Print the two file paths (one per line) so scripts can `open` them. |
| `init [--local]`       | Write a starter policy (read-wide, create narrowly scoped to `AgentWork`). |
| `check --function <f>` | Dry-run a policy check without touching the network. |

`check`'s output distinguishes three outcomes:

- `allowed` — the hypothetical call would go through.
- `denied: <reason>` — `PermissionDenied` (the reason names the
  config file and the matching rule).
- `denied (hidden): <message>` — read-side filter hid the target;
  the agent would see the same response as "not found".

## Reading an item

```sh
# Metadata (field values may be stripped by policy)
zad 1pass get "Deploy Key" --vault AgentWork --json

# One field value (honours deny on vault, item, field, tag, category)
zad 1pass read op://AgentWork/Deploy\ Key/private_key
```

## Injecting a template

```sh
cat <<'EOF' > /tmp/env.tmpl
DB_URL=postgres://bot:op://AgentWork/DB/password@db.internal/prod
API_KEY=op://AgentWork/ThirdParty/api_key
EOF

zad 1pass inject --in /tmp/env.tmpl --out .env
```

Every `op://…` reference is run through the same filter as `read`.
If any reference names a hidden target the whole call fails before
`op` sees the template — no partial renders, no secrets leak.

## Creating an item

```sh
zad 1pass create \
    --vault AgentWork \
    --title "GitHub CI deploy token" \
    --category "API Credential" \
    --tag agent-managed \
    --field username=ci-bot \
    --field "credential=$(head -c 24 /dev/urandom | base64)"
```

The `[create]` block is **deny-by-default**. Without an explicit
`[create].vaults.allow` entry covering `AgentWork`, the call is
refused with `PermissionDenied`. Required tags and allowed categories
enforce a cleaner shape on the agent's writes than `op` itself does.

## See also

- `docs/services.md` — the "Adding a new service" recipe.
- `examples/1pass-permissions/` — a realistic starter policy.
- `man/service.md` — `zad service <action>` lifecycle commands.
