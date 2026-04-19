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
| `SignatureMissing` | File has no `[signature]` block | Run `zad <svc> permissions init` or `zad <svc> permissions sign` |
| `SignatureInvalid` | File was edited after signing, or algorithm/encoding is malformed | The file is tampered. Revert the edit, or re-sign |
| `SignatureKeyMismatch` | File was signed by a different keypair than the one in this machine's keychain | Either import the authoring keypair into the keychain, or re-sign this file with the local one |

## The staged-commit workflow

Agents can **propose** policy changes, but only the user (who controls
the keychain) can **make them enforceable**. Every mutating subcommand
writes to a `<path>.pending` file next to the live policy — unsigned,
so no keychain prompt happens. `commit` is the only step that invokes
the signing key.

### Subcommands (same shape for every service)

| Subcommand | Effect |
|---|---|
| `zad <svc> permissions status [--local]` | Print whether live/pending files exist at the chosen scope. |
| `zad <svc> permissions diff [--local]` | Unified diff of pending vs live. |
| `zad <svc> permissions discard [--local]` | Delete the pending file. Live is untouched. |
| `zad <svc> permissions commit [--local]` | Sign the pending file with the keychain key; atomic rename over live; delete pending. |
| `zad <svc> permissions sign [--local]` | Re-sign the live file in place. Use after a hand edit that broke the signature. |
| `zad <svc> permissions add --function <f> --target <kind> --list allow\|deny <value> [--local]` | Queue a pattern change. |
| `zad <svc> permissions remove --function <f> --target <kind> --list allow\|deny <value> [--local]` | Queue a pattern removal. |
| `zad <svc> permissions content [--function <f>] {add-deny-word\|remove-deny-word\|add-deny-regex\|remove-deny-regex\|set-max-length} ...` | Queue a content-rules change. |
| `zad <svc> permissions time [--function <f>] {set-days --days mon,tue,... \| set-windows --windows 09:00-18:00,...}` | Queue a time-window change. |

`--function` and `--target` are validated against the service's
schema. Example function names per service:

- `discord`: `send`, `read`, `channels`, `join`, `leave`, `discover`, `manage`
- `telegram`: `send`, `read`, `chats`, `discover`
- `gcal`: `list_calendars`, `get_calendar`, `list_events`, `get_event`, `create_event`, `update_event`, `delete_event`, `invite`, `remind`
- `1pass`: `vaults`, `items`, `tags`, `get`, `read`, `inject`, `create`

Example targets per service:

- `discord`: `channel`, `user`, `guild`
- `telegram`: `chat`
- `gcal`: `calendar`, `attendee`
- `1pass`: `vault`, `item`, `tag`, `category`, `field` (plus `title` inside `[create]`)

### Worked example

```sh
# Agent queues a change (no keychain prompt):
zad discord permissions add --function send --target channel \
    --list deny --local 'deploy-*'
# → Queued: [send] channel.deny += "deploy-*"
#   pending: ~/.zad/projects/<slug>/services/discord/permissions.toml.pending

# User reviews:
zad discord permissions diff --local

# User commits (prompts the keychain, signs, atomically replaces live):
zad discord permissions commit --local
```

### Hand-edit escape hatch

If the policy was edited directly with a text editor, the signature
goes stale and every `zad <svc> ...` call fails closed. Re-sign it:

```sh
zad <svc> permissions sign [--local]
```

No mutation — just a fresh signature over the current live contents.

## Rotation

Key rotation is not yet shipped. The `"signing:v1"` account name
leaves room for a future `zad permissions rotate-signing-key` command
that re-signs every `permissions.toml` under `~/.zad/` after
generating a new key.
