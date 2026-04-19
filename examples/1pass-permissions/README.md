# 1Password permissions policy — example

A worked example of a `permissions.toml` for the `1pass` service.
Drop it at one of:

```
~/.zad/services/1pass/permissions.toml                    # global
~/.zad/projects/<slug>/services/1pass/permissions.toml    # local
```

If both files exist they **intersect** — every call must pass every
file that is present. A missing file contributes no restrictions, so
scope is the only gate when neither file exists.

## What the example shows

- Top-level `[vaults]` allow-list scoping the agent to `AgentWork`
  and anything beginning with `Shared-`. Personal vaults and anything
  whose name matches `.*Prod.*` stay invisible.
- `[fields]` deny list that strips field values the agent has no
  business reading (notes, recovery codes, private-key-shaped labels).
  The item still appears in `get` output — just without those field
  values.
- `[tags]` deny list that hides items tagged for a human operator.
- `[categories]` allow list narrowing the agent to the item types it
  typically needs.
- `[content]` rules applied to `inject` output so a template
  accidentally pulling in a credential the filter missed still gets
  caught before it's written to disk.
- `[read]` override that strips even more sensitive fields from
  single-reference reads than the default. `get` still shows their
  labels; `read` treats them as non-existent.
- `[create]` block that restricts writes to the `AgentWork` vault,
  allows only three item categories, and **requires** every created
  item to carry at least one matching tag from the allow list.

## Read-side vs. write-side semantics

**Read-side** (`vaults`, `items`, `tags`, `get`, `read`, `inject`) is
a FILTER. Items or vaults your policy doesn't admit are presented to
the agent as if they don't exist:

```
$ zad 1pass get "Secret Personal Item" --vault Personal
error: 1pass API error: "Secret Personal Item" isn't an item in any
vault visible to this account
```

Same response for a genuinely missing item — no existence leak.

**Write-side** (`create`) is NOT filtered. `PermissionDenied` names
the rule so the agent learns why:

```
$ zad 1pass create --vault Personal --title foo --category Login
error: permission denied for `create`: vault `Personal` did not match
any allow pattern
  config: ~/.zad/services/1pass/permissions.toml
  tip: edit that file (or delete it) to adjust the rule
```

## Try it out

```sh
# Scaffold a global policy from this file.
cp examples/1pass-permissions/permissions.toml \
   ~/.zad/services/1pass/permissions.toml

# Inspect the effective policy.
zad 1pass permissions show

# Dry-run a create against the allowed vault (should succeed).
zad 1pass permissions check --function create \
    --vault AgentWork --category Login --title deploy-token \
    --tag agent-managed

# And against the denied one (should fail).
zad 1pass permissions check --function create \
    --vault Personal --category Login --title anything
```

See [`docs/configuration.md`](../../docs/configuration.md) for the
full schema and [`man/1pass.md`](../../man/1pass.md) for the per-verb
reference.
