# zad ymusic

> Runtime verbs for the YouTube Music service — search the catalogue,
> manage playlists, and curate the user's liked-videos library, all
> gated by a per-verb permissions policy.

## Synopsis

```
zad ymusic <VERB> [OPTIONS]
```

## Description

`zad ymusic` operates the YouTube Music service at runtime. The
project must already have `ymusic` enabled (`zad service enable
ymusic`) and valid Google OAuth credentials registered in either
scope — runtime verbs resolve the effective configuration with local
winning over global, load the three-field OAuth credential
(`client_id` + `client_secret` + `refresh_token`) from the OS
keychain, and mint a fresh access token once per CLI invocation.

YouTube Music has no dedicated API; the runtime client talks to the
**YouTube Data API v3**, which covers playlists, library (rated
videos), and search the same way Spotify Web API v1 covers Spotify.

| Verb | Description |
|---|---|
| `search`                                | Search YouTube (videos / playlists / channels). |
| `playlists list`                        | List the authenticated user's playlists. |
| `playlists show <playlist>`             | Show one playlist's metadata and items. |
| `playlists create <title>`              | Create a new playlist owned by the authenticated user. |
| `playlists rename <playlist> <new>`     | Rename an existing playlist. |
| `playlists delete <playlist>`           | Delete a playlist owned by the user. |
| `playlists add <playlist> <videos…>`    | Add one or more videos to a playlist. |
| `playlists remove <playlist> <items…>`  | Remove one or more items (by playlistItem ID *or* video ID) from a playlist. |
| `library {list,like,unlike}`            | List / like / unlike videos in the user's library. |
| `permissions`                           | Inspect, scaffold, or dry-run the per-project permissions policy. |

Every verb supports `--json` for machine-readable output. Mutating
verbs (`playlists create/rename/delete/add/remove`,
`library like/unlike`) also support `--dry-run` for offline previews.

## Credentials (OAuth 2.0 Desktop app)

YouTube Music uses **Google OAuth 2.0 Desktop app**, identical in
shape to `gcal`. `zad service create ymusic` stores **three** keychain
entries per scope:

- `ymusic-client-id:<scope>`
- `ymusic-client-secret:<scope>`
- `ymusic-refresh:<scope>`

Access tokens are **never persisted**: each CLI invocation exchanges
the refresh token for a fresh access token and uses it for the
lifetime of that process.

### Creating a Google OAuth client

1. Open the Google Cloud Console credentials page:
   `https://console.cloud.google.com/apis/credentials`.
2. Click **Create credentials → OAuth client ID**, type **Desktop
   app**.
3. Enable the **YouTube Data API v3** under **APIs & Services →
   Library**.
4. Run `zad service create ymusic`. zad opens your browser to the
   Google consent screen, accepts the redirect on
   `http://127.0.0.1:<port>`, exchanges the authorization code for a
   refresh token, and stores all three keychain entries.

If you already have a refresh token (minted out-of-band), pass
`--refresh-token` (or `--refresh-token-env`) to skip the browser
flow — useful for CI.

The Google account you authorize must have a **YouTube channel**
attached (most do automatically; if not, `zad service create ymusic`
will warn at validate time and you can create a channel at
`https://youtube.com` before retrying).

## Playlist addressing

Every verb that names a playlist accepts:

- A bare playlist ID: `PLxxxxxxxxxxxxxxxxxxxxxxxxx`
- A YouTube playlist URL: `https://music.youtube.com/playlist?list=PL…`
  or `https://www.youtube.com/playlist?list=PL…`

A `default_playlist` set in the service config is used by
`playlists show` when `--playlist` is omitted.

Video refs in `playlists add/remove` and `library like/unlike` accept
either bare 11-character video IDs or full YouTube watch URLs
(`https://www.youtube.com/watch?v=…`, `https://music.youtube.com/
watch?v=…`, `https://youtu.be/…`); zad normalises every form before
hitting the API.

`playlists remove` accepts both **playlistItem IDs** (the ID of an
entry inside a playlist) and **video IDs** (the canonical YouTube ID
of the video itself). When a video ID is supplied, zad lists the
playlist once, finds every matching item, and removes them all.

## Scope enforcement

Every runtime verb checks the required scope against the `scopes`
array in the effective credentials file **before** any network call.
Missing the scope returns a `scope denied` error that names the exact
file path to edit. The mapping is:

| Verb | Required scope |
|---|---|
| `search`                                          | `search`           |
| `playlists list`, `playlists show`                | `playlists.read`   |
| `playlists create`, `rename`, `delete`, `add`, `remove` | `playlists.write`  |
| `library list`                                    | `library.read`     |
| `library like`, `library unlike`                  | `library.write`    |
| `permissions`                                     | none (local state only) |

Google-side OAuth scopes are computed from the zad scopes at create
time, so the consent screen shows the least possible surface. See
`youtube_scopes_for` in `src/service/ymusic/mod.rs` for the mapping.

## Permissions (second layer)

Scope is the coarse gate — "is this family of operations enabled?".
**Permissions** are the fine gate — "is *this* call (to this target,
at this time, with this content) allowed?". They live in an optional
TOML file at:

- Global: `~/.zad/services/ymusic/permissions.toml`
- Local:  `~/.zad/projects/<slug>/services/ymusic/permissions.toml`

Both files apply — a call must pass every file that exists. Missing
files contribute no restrictions. See
[`docs/configuration.md`](../docs/configuration.md) for the full
schema and [`examples/ymusic-permissions/`](../examples/ymusic-permissions/)
for a worked example.

The schema has five per-verb function blocks (`[search]`,
`[playlists_read]`, `[playlists_write]`, `[library_read]`,
`[library_write]`), each with a single `targets` allow / deny list,
optional content rules, and an optional time window. Top-level
`[content]` and `[time]` sections cascade into every block as
defaults.

```toml
[content]
deny_words = ["password", "api_key"]
max_length = 256

[time]
days    = ["mon","tue","wed","thu","fri"]
windows = ["09:00-22:00"]

[playlists_write]
targets.allow = ["zad-*", "scratch-*"]
targets.deny  = ["*release*", "*official*"]

[library_write]
targets.deny = ["dQw4w9WgXcQ"]
```

## Examples

```sh
# Search for a track / video
zad ymusic search "moon river"

# Search across multiple types
zad ymusic search "kind of blue" --type playlist --type channel

# Manage playlists
zad ymusic playlists list
zad ymusic playlists create "zad-test" --description "from agent"
zad ymusic playlists add zad-test dQw4w9WgXcQ
zad ymusic playlists show zad-test
zad ymusic playlists remove zad-test dQw4w9WgXcQ
zad ymusic playlists delete zad-test

# Library management
zad ymusic library list --limit 5
zad ymusic library like dQw4w9WgXcQ
zad ymusic library unlike dQw4w9WgXcQ

# Inspect permissions
zad ymusic permissions show
zad ymusic permissions check --function playlists_write --target some-playlist

# Preview a write without hitting YouTube
zad ymusic playlists create "preview" --dry-run
```

## See also

- `zad service create ymusic` — register credentials.
- `zad service enable ymusic` — opt the current project into using
  them.
- `man docs/configuration.md` — the full YouTube Music config and
  permissions schema.
- `examples/ymusic-permissions/` — a worked example of a real policy.
