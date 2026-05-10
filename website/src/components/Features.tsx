import { services } from "../data/sourceData";

const features = [
  {
    title: `${services.length} services, one config shape`,
    description:
      "Discord, Slack, Telegram, Google Calendar, Spotify, YouTube Music, and 1Password all live behind the same `zad service create | enable | disable | show | status` lifecycle. Adding GitHub, Matrix, or Reddit is a checklist, not a rewrite.",
    icon: "🧩",
  },
  {
    title: "No MCP server to run",
    description:
      "MCP needs a long-running server per agent — and per machine, per restart. zad ships a single binary the agent shells out to. No daemon, no port, no socket, no race conditions.",
    icon: "🪶",
  },
  {
    title: "Signed, scoped permissions",
    description:
      "Two layers: scopes gate which families of operations are enabled, permissions narrow each call by channel, user, time window, and content. Files are signed with an Ed25519 key in your OS keychain — agents can propose changes, but only you can commit them.",
    icon: "🔐",
  },
  {
    title: "Global + project-local, strictest wins",
    description:
      "Drop a baseline at `~/.zad/services/<svc>/permissions.toml` and tighter overrides per repo. Both files apply at once; an agent in a project can never loosen the global rule.",
    icon: "🪜",
  },
  {
    title: "Dry-run every mutation",
    description:
      "`--dry-run` previews any send / create / update before a single byte hits the network. Scopes and permissions still fire — you can hand the output to a reviewer with confidence.",
    icon: "🪞",
  },
  {
    title: "Typed Rust facade, no string slop",
    description:
      "Embed `zad` as a library and you get `Discord::send`, `SendRequest::new`, newtypes for `ChannelId` / `UserId` / `MessageId`. Wrong-shape calls fail at construction with `ZadError::Invalid` — never at the wire.",
    icon: "🦀",
  },
  {
    title: "OS keychain for every secret",
    description:
      "Bot tokens and OAuth refresh tokens live in macOS Keychain, Linux Secret Service, or Windows Credential Manager — never in TOML, never in env vars by default. Tests use an in-memory backend.",
    icon: "🗝️",
  },
  {
    title: "OAuth that just works",
    description:
      "PKCE loopback flows for Google Calendar, Spotify, and YouTube Music. `zad service create` opens your browser, captures the redirect, exchanges the code, and persists the refresh token — interactive or fully headless.",
    icon: "🌀",
  },
  {
    title: "JSON output everywhere",
    description:
      "Every command takes `--json` for stable, parseable envelopes. `zad commands --json` dumps the entire CLI tree so an agent (or this very website) can introspect it without invoking `--help`.",
    icon: "📦",
  },
  {
    title: "--help-agent, not just --help",
    description:
      "Each command exposes a self-describing prompt the agent can ingest before invoking it. No more guessing flags from man-page text.",
    icon: "🤖",
  },
  {
    title: "Status check across every service",
    description:
      "`zad service status` pings every configured provider in one shot, returns a stable JSON envelope, and reflects success in its exit code. Drop it in a cron job, see drift the moment it happens.",
    icon: "💓",
  },
  {
    title: "Library + CLI, lockstep versions",
    description:
      "Two crates ship from one workspace: `zad` (library) and `zad-cli` (binary). The CLI is a thin wrapper, so behaviour can't drift. Pick whichever shape matches your integration.",
    icon: "📚",
  },
];

export default function Features() {
  return (
    <section id="features" className="border-t border-border py-20 md:py-28">
      <div className="mx-auto max-w-6xl px-6">
        <h2 className="text-center text-3xl font-bold text-text-primary md:text-4xl">
          What zad gives you
        </h2>
        <p className="mx-auto mt-4 max-w-2xl text-center text-text-secondary">
          A focused tool for one job: letting agents act on your behalf in
          real services, without the per-agent server sprawl that MCP creates
          and without surrendering authorisation to a UI checkbox you can't audit.
        </p>

        <div className="mt-14 grid gap-6 sm:grid-cols-2 lg:grid-cols-3">
          {features.map((f) => (
            <div
              key={f.title}
              className="group rounded-xl border border-border bg-surface-alt p-6 transition-all hover:border-accent/40 hover:bg-surface-hover"
            >
              <div className="mb-4 text-2xl" aria-hidden>
                {f.icon}
              </div>
              <h3 className="mb-2 text-lg font-semibold text-text-primary">{f.title}</h3>
              <p className="text-sm leading-relaxed text-text-secondary">{f.description}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
