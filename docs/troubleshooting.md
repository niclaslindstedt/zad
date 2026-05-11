# Troubleshooting

Common failure modes and how to fix them. Each entry should have:

- **Symptom** — what the user sees
- **Cause** — what's actually wrong
- **Fix** — how to resolve it
- **Prevention** — how to avoid it next time

## Rate limits (HTTP 429)

- **Symptom** — a service-bound subcommand exits non-zero with
  `<service> rate-limited this call (HTTP 429); wait Ns (until
  <UTC>). Re-run the same command with --wait …`. Under `--json`,
  stdout contains `{"error": "rate_limited", "service": …,
  "retry_after_seconds": …, "retry_after_utc": …, …}`.
- **Cause** — the provider returned HTTP 429 (or its body-only
  equivalent, e.g. Slack's `error: "ratelimited"`). zad parses
  `Retry-After`, persists the deadline at
  `~/.zad/state/<service>/rate_limit.json`, and surfaces the typed
  error.
- **Fix** — re-run the exact same command with `--wait` to block
  until the deadline passes and then proceed. To script around it
  without sleeping, read `retry_after_seconds` from the JSON
  payload, sleep that many seconds, and retry — `--wait` does this
  for you.
- **Prevention** — leave `--wait` on permanently in scripts; it is a
  no-op when no wait window is active. For Spotify specifically,
  prefer batch endpoints (`Get Multiple Albums`, `Get Multiple
  Tracks`) and the `snapshot_id` shortcut for playlists over
  high-frequency polling — see
  <https://developer.spotify.com/documentation/web-api/concepts/rate-limits>.
  The provider does not expose a remaining-quota header, so honouring
  the persisted deadline across processes is the most reliable
  client-side avoidance.

### Limitations

- For Discord, serenity's `ErrorResponse` does not preserve the
  `Retry-After` header, so a persistent 429 falls back to a short
  default wait. Serenity also performs its own in-process bucket
  retries before bubbling 429 up to zad, so this path is rare.
