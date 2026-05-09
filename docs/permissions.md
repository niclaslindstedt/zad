# Permissions signing

Every permission file under `~/.zad/services/<svc>/permissions.toml`
and its per-project local equivalent is **policy only** — the file
itself carries no signature. Authorization to load it lives in a
per-machine **trust store** at `~/.zad/signing/trusted.toml`, which
is itself signed by the OS-keychain Ed25519 key. Load-time
verification fails closed: a missing trust entry, a tampered file, or
a tampered trust store is a `PermissionDenied` error, not a warning.

This guarantees that permission policies are **tamper-evident** *and*
**shippable**: a project can check `permissions.toml` into a repo and
each operator runs `zad <svc> permissions sign` to add the file to
their own trust store. The file body is identical across machines;
only the trust store is local.

## The signing key

`zad` maintains **one** signing keypair per installation, stored in
the OS keychain under the account name `signing:v1`:

- **macOS** — Keychain Access, service `zad`, account `signing:v1`
- **Linux** — Secret Service (libsecret), attribute
  `service=zad`, `account=signing:v1`
- **Windows** — Credential Manager, target name `zad/signing:v1`

The private key never leaves the keychain — zad reads it only when it
needs to sign. The public key is cached at
`~/.zad/signing/public_key.toml` for visibility; the cache is **not**
consulted during verification (the keychain is always authoritative).

### Bootstrap

The signing key is bootstrapped by an explicit, single-purpose
command:

```sh
zad signing init
```

This is the only code path that mints a fresh keypair. Routine
`permissions sign` and `permissions commit` paths fail closed with
`SigningKeyMissing` when the keychain is empty, pointing the operator
at `zad signing init`. The OS keychain prompt that the bootstrap
triggers is the user-presence gate that prevents an agent from
silently minting its own root of trust.

Rotation:

```sh
zad signing init --force
```

This rotates the keychain key and resets the trust store. Every
permissions file you want loaded must be re-signed with `zad <svc>
permissions sign`.

## Trust model

When `zad` loads a permission file:

1. The OS keychain must contain a signing key. If not →
   `SigningKeyMissing`. Bootstrap with `zad signing init`.
2. The trust store at `~/.zad/signing/trusted.toml` is loaded and its
   `[signature]` block is verified against the keychain pubkey. A
   tamper (mismatched signature, mismatched pubkey, symlink at the
   path) is `TrustStoreTampered`.
3. The trust store is searched for an entry keyed by the canonical
   absolute path of the file being loaded. No entry → `NotTrusted`.
4. The entry's signature is verified against the canonical bytes of
   the file's contents. A mismatch (file edited after signing) is
   `SignatureInvalid`.
5. The entry's pubkey must match the keychain pubkey. A mismatch
   (entry signed by a previous, since-rotated key) is
   `SignatureKeyMismatch`.

The OS keychain is the single root of trust. Anyone who can read the
keychain signing key can forge any signature; anyone who cannot
cannot, even with full read/write access to `~/.zad/`.

## Failure modes

| Error | Cause | Fix |
|---|---|---|
| `SigningKeyMissing` | OS keychain has no signing key | Run `zad signing init` |
| `NotTrusted` | The file at this path has no entry in the trust store | Run `zad <svc> permissions sign [--local]` |
| `SignatureInvalid` | File was edited after signing, or trust-store entry's encoding is malformed | Revert the edit, or re-sign with `zad <svc> permissions sign` |
| `SignatureKeyMismatch` | Trust-store entry was signed by a previous keychain key | Re-sign every permissions file with the rotated key (`zad <svc> permissions sign`), or restore the previous keychain entry |
| `TrustStoreTampered` | `~/.zad/signing/trusted.toml` is a symlink, has a broken self-signature, or was signed by an unknown key | Run `zad signing init --force` to rotate and rebuild; you will need to re-sign every permissions file |

## Echo mode for runtime verbs

Runtime verbs (`zad discord send`, `zad slack send`, `zad telegram
send`, `zad gcal events create`, …) treat the five errors above
specially: instead of issuing the network call, the CLI **echoes the
call that would have run** along with the reason, then exits `3`.
That makes it possible to iterate on an unsigned permissions file
without round-tripping through real API calls.

Mechanism (no flag, always on):

1. `load_effective` returns one of the signing errors above.
2. The verb arms echo mode and falls through to the same dry-run
   transport that powers `--dry-run`. Permission rule checks (content,
   time, pattern) are skipped — they would be evaluated against an
   untrustworthy file.
3. Argument-level structural checks (channel snowflake parse, body
   length cap, attachment count cap) still run. A failure there is a
   normal `ZadError` (exit `1`).
4. The verb's success print site swaps for an echo envelope and
   `mark_echoed()` is set so `main.rs` returns exit code `3`.

Output:

- Human (`zad discord send --channel 12345 hi`):
  ```
  would have run: would send 2 chars to channel 12345
    reason: permission denied for `load`: not trusted
  ```
- JSON (`--json`):
  ```json
  {
    "echoed": {
      "command": "discord.send",
      "target": "channel",
      "target_id": "12345",
      "body": "hi",
      ...
    },
    "error": {
      "kind": "not_trusted",
      "reason": "permission denied for `load`: not trusted ...",
      "path": "/Users/me/.zad/projects/.../services/discord/permissions.toml"
    }
  }
  ```

`error.kind` is one of `not_trusted`, `signature_invalid`,
`signature_key_mismatch`, `trust_store_tampered`,
`signing_key_missing` — stable across releases so callers can switch
on it.

Diagnostic verbs (`permissions show|check|path|init|stage|commit|...`)
do **not** participate in echo mode: signing errors there exit `1`
with the underlying `ZadError`, because those subcommands exist to
diagnose and fix a broken trust state.

Exit codes summary:

| Code | Meaning |
|---|---|
| `0` | The call ran (or `--dry-run` previewed it). |
| `1` | A non-signing error stopped the call (rule denial, parse, I/O, scope, etc.). |
| `3` | Permissions could not be trusted; the call was echoed instead of issued. |

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
| `zad <svc> permissions commit [--local]` | Atomically replace live with pending and upsert a trust-store entry signed with the keychain key. |
| `zad <svc> permissions sign [--local]` | Sign the live file with the keychain key and add (or update) its trust-store entry. Use after a hand edit, or to trust a permissions file shipped from another machine. |
| `zad <svc> permissions add --function <f> --target <kind> --list allow\|deny <value> [--local]` | Queue a pattern change. |
| `zad <svc> permissions remove --function <f> --target <kind> --list allow\|deny <value> [--local]` | Queue a pattern removal. |
| `zad <svc> permissions content [--function <f>] {add-deny-word\|remove-deny-word\|add-deny-regex\|remove-deny-regex\|set-max-length} ...` | Queue a content-rules change. |
| `zad <svc> permissions time [--function <f>] {set-days --days mon,tue,... \| set-windows --windows 09:00-18:00,...}` | Queue a time-window change. |

`--function` and `--target` are validated against the service's
schema. Example function names per service:

- `discord`: `send`, `read`, `channels`, `join`, `leave`, `discover`, `manage`
- `slack`: `send`, `read`, `channels`, `discover`
- `telegram`: `send`, `read`, `chats`, `discover`
- `gcal`: `list_calendars`, `get_calendar`, `list_events`, `get_event`, `create_event`, `update_event`, `delete_event`, `invite`, `remind`
- `spotify`: `search`, `playlists_read`, `playlists_write`, `library_read`, `library_write`
- `ymusic`: `search`, `playlists_read`, `playlists_write`, `library_read`, `library_write`
- `1pass`: `vaults`, `items`, `tags`, `get`, `read`, `inject`, `create`

Example targets per service:

- `discord`: `channel`, `user`, `guild`
- `slack`: `channel`, `user`, `workspace`
- `telegram`: `chat`
- `gcal`: `calendar`, `attendee`
- `spotify`, `ymusic`: `playlist`, `track`, `album`, `query`
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
