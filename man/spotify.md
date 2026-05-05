# zad spotify

> Runtime verbs for the Spotify service — search the catalogue,
> manage playlists, and curate the user's saved-tracks / saved-albums
> library, all gated by a per-verb permissions policy.

## Synopsis

```
zad spotify <VERB> [OPTIONS]
```

## Description

`zad spotify` operates the Spotify service at runtime. The project
must already have `spotify` enabled (`zad service enable spotify`)
and valid OAuth credentials registered in either scope — runtime
verbs resolve the effective configuration with local winning over
global, load the two-field OAuth credential (`client_id` +
`refresh_token`) from the OS keychain, and mint a fresh access token
once per CLI invocation.

| Verb | Description |
|---|---|
| `search`                       | Search the Spotify catalogue (tracks / albums / artists / playlists). |
| `playlists list`               | List the authenticated user's playlists. |
| `playlists show <playlist>`    | Show one playlist's metadata and tracks. |
| `playlists create <name>`      | Create a new playlist owned by the authenticated user. |
| `playlists rename <playlist> <new>` | Rename an existing playlist. |
| `playlists delete <playlist>`  | Delete (i.e. unfollow) a playlist owned by the user. |
| `playlists add <playlist> <tracks…>`    | Add one or more tracks to a playlist. |
| `playlists remove <playlist> <tracks…>` | Remove one or more tracks from a playlist. |
| `library tracks {list,save,unsave}`     | List / save / unsave saved (liked) tracks. |
| `library albums {list,save,unsave}`     | List / save / unsave saved (liked) albums. |
| `permissions`                           | Inspect, scaffold, or dry-run the per-project permissions policy. |

Every verb supports `--json` for machine-readable output.

## Credentials (OAuth 2.0 PKCE public client)

Spotify uses **OAuth 2.0 Authorization Code with PKCE** for desktop /
CLI apps, which is a *public client* — no `client_secret` is issued
or accepted. `zad service create spotify` stores **two** keychain
entries per scope:

- `spotify-client-id:<scope>`
- `spotify-refresh:<scope>`

Access tokens are **never persisted**: each CLI invocation exchanges
the refresh token for a fresh access token and uses it for the
lifetime of that process.

### Creating a Spotify app

1. Open the Spotify Developer Dashboard:
   `https://developer.spotify.com/dashboard`.
2. Click **Create app**. Name and description are arbitrary.
3. Under **Redirect URIs**, add `http://127.0.0.1` and save. zad's
   loopback listener picks a random port, but Spotify accepts any
   port on `127.0.0.1` once the host is registered.
4. Run `zad service create spotify`. zad opens your browser to
   Spotify's consent screen, accepts the redirect on
   `http://127.0.0.1:<port>`, exchanges the authorization code for a
   refresh token with PKCE (S256), and stores both keychain entries.

The Spotify dashboard *also* shows a Client Secret — **you do not
need it** for the PKCE flow. zad stores only the Client ID + the
refresh token.

If you already have a refresh token (minted out-of-band), pass
`--refresh-token` (or `--refresh-token-env`) to skip the browser
flow — useful for CI.

## Playlist addressing

Every verb that names a playlist accepts:

- A bare ID: `5xR…`
- A Spotify URI: `spotify:playlist:5xR…`
- A directory alias: `my-pl`

A `default_playlist` set in the service config is used by `playlists
show` when `--playlist` is omitted.

Track and album refs in `playlists add/remove` and `library
tracks/albums save/unsave` accept either bare IDs or full
`spotify:track:<id>` / `spotify:album:<id>` URIs; zad normalises both
forms before hitting the API.

## Scope enforcement

Every runtime verb checks the required scope against the `scopes`
array in the effective credentials file **before** any network call.
Missing the scope returns a `scope denied` error that names the exact
file path to edit. The mapping is:

| Verb | Required scope |
|---|---|
| `search`                                    | `search` |
| `playlists list`, `playlists show`          | `playlists.read` |
| `playlists create`, `rename`, `delete`, `add`, `remove` | `playlists.write` |
| `library tracks list`, `library albums list` | `library.read` |
| `library tracks save/unsave`, `library albums save/unsave` | `library.write` |
| `permissions`                               | none (local state only) |

Spotify-side OAuth scopes are computed from the zad scopes at create
time, so the consent screen shows the least possible surface. See
`spotify_scopes_for` in `src/service/spotify/mod.rs` for the mapping.

## Permissions (second layer)

Scope is the coarse gate — "is this family of operations enabled?".
**Permissions** are the fine gate — "is *this* call (to this target,
at this time, with this content) allowed?". They live in an optional
TOML file at:

- Global: `~/.zad/services/spotify/permissions.toml`
- Local: `~/.zad/projects/<slug>/services/spotify/permissions.toml`

Both files apply — a call must pass every file that exists. Missing
files contribute no restrictions. See
[`docs/configuration.md`](../docs/configuration.md) for the full
schema and [`examples/spotify-permissions/`](../examples/spotify-permissions/)
for a worked example.

The schema has five per-verb function blocks (`[search]`,
`[playlists_read]`, `[playlists_write]`, `[library_read]`,
`[library_write]`), each with a single `targets` allow / deny list,
optional content rules, and an optional time window. Top-level
`[content]` and `[time]` sections cascade into every block as
defaults.

```toml
[content]
deny_words    = ["password", "api_key"]
max_length    = 256

[time]
days    = ["mon","tue","wed","thu","fri"]
windows = ["09:00-22:00"]

[playlists_write]
targets.allow = ["zad-*", "scratch-*"]
targets.deny  = ["*release*", "*official*"]

[library_write]
targets.deny = ["spotify:track:5p..."]
```

## Examples

```sh
# Search for a track
zad spotify search "moon river"

# Search across multiple types
zad spotify search "kind of blue" --type album --type artist

# Manage playlists
zad spotify playlists list
zad spotify playlists create "zad-test" --description "from agent"
zad spotify playlists add zad-test spotify:track:5p...
zad spotify playlists show zad-test
zad spotify playlists remove zad-test spotify:track:5p...
zad spotify playlists delete zad-test

# Library management
zad spotify library tracks list --limit 5
zad spotify library tracks save spotify:track:5p...
zad spotify library tracks unsave spotify:track:5p...
zad spotify library albums list --limit 5

# Inspect permissions
zad spotify permissions show
zad spotify permissions check --function playlists_write --target some-playlist
```

## See also

- `zad service create spotify` — register credentials.
- `zad service enable spotify` — opt the current project into using
  them.
- `man docs/configuration.md` — the full Spotify config and
  permissions schema.
- `examples/spotify-permissions/` — a worked example of a real
  policy.
