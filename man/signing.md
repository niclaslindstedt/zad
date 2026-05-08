# zad signing

> Manage the local Ed25519 signing key and the per-machine trust store
> that authorizes permission file loads.

## Synopsis

```
zad signing init [--force] [--json]
zad signing show [--json]
```

## Description

zad's permission files (`~/.zad/services/<service>/permissions.toml`,
`~/.zad/projects/<slug>/services/<service>/permissions.toml`, and any
file pointed to by `ZAD_PERMISSIONS_PATH` /
`ZAD_PERMISSIONS_ROOT`) are policy-only TOML — they do **not** carry
their own signatures. Authorization lives in the per-machine trust
store at `~/.zad/signing/trusted.toml`, which is itself signed by the
local Ed25519 signing key in the OS keychain.

This command group is the **single bootstrap point** for that key. No
other command silently mints a signing key; routine `permissions
sign` / `permissions commit` paths fail closed with `SigningKeyMissing`
when the keychain is empty, pointing the operator at `zad signing
init`.

## Subcommands

### `init [--force]`

Create the local signing key and an empty signed trust store.

* On a fresh machine: mints a fresh Ed25519 keypair, stores the
  private scalar in the OS keychain (account `signing:v1`), writes the
  public-key cache at `~/.zad/signing/public_key.toml`, and writes an
  empty self-signed `~/.zad/signing/trusted.toml`. Prints the new
  key's fingerprint.
* On a machine that already has a key: idempotent — prints the
  existing key's fingerprint, leaves both the keychain and the trust
  store alone.
* `--force`: rotate the keychain key. Discards the existing trust
  store (every entry was signed by the previous key and would fail
  verification under the new one); you must re-sign every permissions
  file you want loaded with `zad <service> permissions sign`.

### `show`

Print the current signing key's fingerprint and the on-disk paths of
the trust store and the public-key cache. Useful for confirming the
current trust state before signing or rotating.

## Threat model

The OS keychain is the **single root of trust**. Anyone who can read
the keychain signing key can forge any signature; anyone who cannot
cannot, even with full read/write access to `~/.zad/`.

Defenses around the trust store:

- It is signed by the keychain key, so an agent that rewrites it with
  its own keypair fails the keychain cross-check on the next load.
- Verification refuses to proceed without the keychain key — there is
  no embedded-pubkey fallback in production.
- The store path is fixed; no env-var override redirects verification
  to an attacker-controlled file.
- Symlinks at the store path are refused.
- On Unix the store is written with mode `0o600`.

## Environment variables

| Variable | Description |
|---|---|
| `ZAD_HOME_OVERRIDE` | Resolves `~` for `~/.zad/`. Tests only. |
| `ZAD_SECRETS_MEMORY` | When `1`, route the keychain through a process-local map. Tests only — the OS keychain is the production gate. |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | I/O error or invalid input |
| 2 | tamper detected (`TrustStoreTampered`) |

## Examples

```sh
# Bootstrap a fresh machine. The OS keychain prompts on first key access.
zad signing init

# Show the current key fingerprint.
zad signing show

# Rotate the key (destructive: invalidates the trust store).
zad signing init --force
# Re-sign each permissions file you want loaded:
zad discord permissions sign --local
zad telegram permissions sign --local
# … etc per service.
```

## See also

- [`zad man discord`](discord.md), [`zad man telegram`](telegram.md), [`zad man slack`](slack.md), [`zad man gcal`](gcal.md), [`zad man 1pass`](1pass.md), [`zad man spotify`](spotify.md) — each service's `permissions sign` and `permissions commit` are the call sites that consume the signing key.
