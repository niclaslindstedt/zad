# GitHub permissions policy — example

A worked example of a `permissions.toml` for the GitHub service. Drop
it at one of:

```
~/.zad/services/github/permissions.toml                       # global
~/.zad/projects/<slug>/services/github/permissions.toml       # local
```

If both files exist they **intersect** — every call must pass every
file that is present. A missing file contributes no restrictions, so
scope is the only gate when neither file exists.

## What the example shows

- Shared `[content]` defaults that deny obvious credential shapes
  (classic PATs, fine-grained PATs, bearer tokens) and cap outbound
  body length.
- Broad-but-curated read-verb allow-lists so the agent can read across
  your orgs while still blocking known-sensitive repos
  (`*/secrets-*`, `*/vault-*`).
- Tight write-verb allow-lists so the agent can open issues/PRs only
  against explicitly-listed repos, never production repos.
- `[pr_merge]` default-deny — the always-tightest block; edit the
  allow list deliberately per repo.

## Try it out

```sh
# Scaffold a project-local policy (signed with the local signing key).
zad github permissions init --local

# Or copy this example verbatim:
cp examples/github-permissions/permissions.toml \
   ~/.zad/projects/<slug>/services/github/permissions.toml
zad github permissions sign --local

# Inspect the effective policy.
zad github permissions show

# Dry-run a specific call without shelling out to `gh`.
zad github permissions check --function pr_merge --repo myorg/webapp
zad github permissions check --function issue_comment \
    --repo myorg/webapp --body "triaging"
```

Permission violations surface with a `permission denied` error that
names the function, the reason, and the exact file path to edit — the
same shape as the scope-denied error. Permission checks fire
**before** any `gh` subprocess spawn, so a denied call never reaches
the network.

See [`docs/configuration.md`](../../docs/configuration.md#permissions-file)
for the full schema and the
[`man/github.md`](../../man/github.md) page for the full verb → block
mapping.
