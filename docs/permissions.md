# Permissions signing

Every permission file under `~/.zad/services/<svc>/permissions.toml`
and its per-project local equivalent carries an Ed25519 signature over
the canonical serialization of its contents. Load-time verification
fails closed: a missing, malformed, or mismatched signature is a
`PermissionDenied` error, not a warning.

This guarantees that permission policies are **tamper-evident**. An
agent with filesystem access cannot silently loosen a policy (e.g.,
add an allow-list exception, raise `max_length`) and have the change
take effect — the next `zad` call will refuse to load the file.

## The signing key

`zad` maintains **one** signing keypair per installation, stored in
the OS keychain under the account name `signing:v1`:

- **macOS** — Keychain Access, service `zad`, account `signing:v1`
- **Linux** — Secret Service (libsecret), attribute
  `service=zad`, `account=signing:v1`
- **Windows** — Credential Manager, target name `zad/signing:v1`

The private key never leaves the keychain — zad reads it only when it
needs to sign. The **public** key is embedded in every signed file,
and also cached at `~/.zad/signing/public_key.toml` so tooling can
verify offline without prompting the keychain.

### First-run (TOFU)

The first `zad <svc> permissions init` on a fresh machine generates
a fresh keypair, stores it in the keychain, caches the public key,
and signs the starter template. The keypair's short fingerprint is
printed so the user can verify it later (e.g., pin it in a team
secrets manager).

Subsequent `init` calls reuse the same keypair.

## Trust model

When `zad` loads a permission file:

1. The file's embedded `[signature]` block must be present, well-formed,
   and verify against the file's payload (everything except the
   `[signature]` block itself, re-serialized canonically).
2. If the local keychain holds a signing key, its public key **must
   match** the one embedded in the file. A mismatch is a hard fail —
   this is what prevents an attacker from rewriting both the policy
   body and the embedded pubkey.
3. If the local keychain holds no signing key (e.g., an agent running
   on a fresh machine that has never run `zad permissions init`), the
   file's embedded pubkey is authoritative. This is safe because
   without the private key no one can forge a new signature.

## Failure modes

| Error | Cause | Fix |
|---|---|---|
| `SignatureMissing` | File has no `[signature]` block | Run `zad <svc> permissions init` (to regenerate) or — once staged edits land in PR 2 — `zad <svc> permissions sign` |
| `SignatureInvalid` | File was edited after signing, or algorithm/encoding is malformed | The file is tampered. Revert the edit, or re-sign |
| `SignatureKeyMismatch` | File was signed by a different keypair than the one in this machine's keychain | Either import the authoring keypair into the keychain, or re-sign this file with the local one |

## Editing a permission file by hand

Do not edit the `[signature]` block. To change a policy, edit the
other fields and then re-sign. The canonical workflow (which ships
in PR 2) will be:

```sh
zad <svc> permissions add --function send --target channel --deny 'admin-*'
zad <svc> permissions diff     # review queued change
zad <svc> permissions commit   # signs and atomically replaces live file
```

Until PR 2 lands, hand edits are still possible: edit the file, then
re-initialize with `--force` (which re-signs).

## Rotation

Key rotation is not yet shipped. The `"signing:v1"` account name
leaves room for a future `zad permissions rotate-signing-key` command
that re-signs every `permissions.toml` under `~/.zad/` after
generating a new key.
