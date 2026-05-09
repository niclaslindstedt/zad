# Discord library — example

A minimal Rust crate that depends on `zad` and sends a Discord message
through the typed library API instead of shelling out to the `zad`
binary. Demonstrates the post-0.7 split: `zad-cli` is the binary
package; `zad` is the library you embed.

## Layout

```
discord-library/
├── Cargo.toml      # zad = "0.7" pinned dependency
└── src/main.rs     # Discord::from_default_config() + SendRequest
```

## What this shows

1. **Pinned dependency.** A downstream project writes
   `zad = "0.7"` in its `Cargo.toml` and gets the typed API. The
   binary is not required at runtime; the program links the library
   directly.
2. **Typed inputs.** `SendRequest::new(target, body, attachments)`
   validates the body length, attachment count, and the empty-body /
   no-attachment combination at construction time. Wrong-shape calls
   surface as `zad::ZadError::Invalid` before any network I/O — they
   never become silent runtime failures.
3. **Typed outputs.** `Discord::send` returns a `SendResponse` whose
   `message_id` is a `MessageId(u64)` newtype. The caller doesn't
   parse stdout, doesn't shell out, doesn't deserialize JSON.
4. **Same code path as the CLI.** `Discord::from_default_config()`
   loads the project-local config (or falls back to the global one)
   and the bot token from the OS keychain — exactly what
   `zad discord send` does. So the CLI and library can't drift; both
   call the same library functions.

## Running it

This example is **not** part of the workspace's regular build
(`publish = false` keeps it out of release artefacts). Run it
on-demand against a project that already has Discord configured:

```sh
# 1. configure Discord credentials once (uses the CLI):
zad service create discord
zad service enable discord

# 2. edit src/main.rs — set CHANNEL_ID to a real channel snowflake.

# 3. run:
cargo run --manifest-path examples/discord-library/Cargo.toml
```

A successful send prints:

```
sent message 1234567890123456789 to channel 123456789012345678
```

## Trying the type-level safety regression

Replace the `Target::Channel(ChannelId(...))` line with a `UserId`
construction and observe the compiler refuse the wrong newtype:

```rust
// Won't compile — Target::Channel takes ChannelId, not UserId.
Target::Channel(zad::service::UserId(CHANNEL_ID))
```

Or push the body past Discord's limit and watch validation reject
the request before any network call:

```rust
let body = "x".repeat(2001);
SendRequest::new(channel, MessageBody::text(body), vec![])?;
//                                                          ^ ZadError::Invalid
```
