# Changelog

## [0.1.0]

- chore(deps): bump actions/checkout from 4 to 6 (#2)
- chore(deps): bump actions/setup-node from 4 to 6 (#3)
- chore(release): update changelog and versions for v0.1.0
- chore: bootstrap project from oss-spec
- ci(pages): auto-enable Pages site in configure-pages (#36)
- ci(pages): build zad binary before website extractor (#35)
- ci: expand release matrix and align workflows with workspace sync
- docs: add generic services architecture doc (#14)
- docs: drift sweep — 1pass and gcal across manpages, README, architecture (#37)
- docs: fix all OSS_SPEC §3/4/7/8/10.3 conformance violations (#7)
- docs: sync OSS_SPEC.md to v2.3.0
- docs: sync manpages, architecture doc, and README with Telegram service (#22)
- feat(adapter): add Discord adapter with create/add commands (#6)
- feat(adapter): add list, show, delete commands (#8)
- feat(cli)!: rename `adapter add` to `enable`, add `disable`, add `--json` (#9)
- feat(discord): add --dry-run to send/join/leave via generic transport trait (#16)
- feat(discord): add permissions layer with global+local policy files (#13)
- feat(discord): deep-link portal + OAuth install URL on create (#19)
- feat(discord): runtime verbs + name → snowflake directory (#11)
- feat(permissions): sign permission files with Ed25519, verify on load (#33)
- feat(permissions): stage mutations in .pending files; commit signs + replaces (#34)
- feat(service): add 1Password (1pass) via `op` CLI with filter-style permissions (#32)
- feat(service): add Google Calendar (gcal) with OAuth 2.0 + rich permissions (#28)
- feat(service): add file attachments to discord and telegram send (#31)
- feat(service): add status check that pings providers with stored credentials (#23)
- feat(service): link users to provider portals during interactive create (#24)
- feat(service): resolve `@me` in send targets via captured self identity (#25)
- feat(telegram): add service with live lifecycle and stubbed runtime verbs (#20)
- feat(telegram): implement send/read/chats/discover runtime verbs (#21)
- feat: close every OSS_SPEC.md v2.0.1 conformance gap (#15)
- fix(discord): enforce scopes, validate message length and --limit, typed 403/404 errors (#12)
- refactor!: generalize service scaffolding for any-service support (#18)
- refactor!: rename `adapter` to `service` throughout (#10)
- refactor(service)!: fold `zad status` into `zad service status [--service &lt;name&gt;]` (#26)

