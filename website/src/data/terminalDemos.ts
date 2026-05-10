// Terminal demo scripts. Each tab plays a sequence of lines back as if a
// shell is running them; lines have a `kind` ("prompt" | "output" |
// "comment") so the renderer can style them differently.

export type LineKind = "prompt" | "output" | "comment";

export interface DemoLine {
  kind: LineKind;
  text: string;
  /** Output dwell after the line is fully typed/printed, in ms. */
  delayAfter?: number;
}

export interface TerminalDemo {
  id: string;
  label: string;
  prompt: string;
  lines: DemoLine[];
}

export const terminalDemos: TerminalDemo[] = [
  {
    id: "discord",
    label: "Discord",
    prompt: "~/code/my-project $ ",
    lines: [
      { kind: "comment", text: "# One-time: register a bot once, share it across every project." },
      { kind: "prompt",  text: "zad service create discord --application-id 1234567890" },
      { kind: "output",  text: "✓ Token stored in OS keychain.", delayAfter: 250 },
      { kind: "output",  text: "✓ Opening OAuth install URL in your browser…", delayAfter: 600 },
      { kind: "comment", text: "# Per-project: opt in." },
      { kind: "prompt",  text: "zad service enable discord" },
      { kind: "output",  text: "✓ discord enabled in this project.", delayAfter: 400 },
      { kind: "comment", text: "# Talk to it by name, not by snowflake." },
      { kind: "prompt",  text: "zad discord send --channel deploys 'shipped v0.6.5 ✨'" },
      { kind: "output",  text: "→ message 1336124488907980800 sent in #deploys", delayAfter: 800 },
    ],
  },
  {
    id: "permissions",
    label: "Permissions",
    prompt: "~/code/my-project $ ",
    lines: [
      { kind: "comment", text: "# Strictest of (global, project) wins. Local can never loosen global." },
      { kind: "prompt",  text: "zad discord permissions check --function send \\" },
      { kind: "prompt",  text: "    --channel deploy-prod --body 'shipping...'" },
      { kind: "output",  text: "PermissionDenied: send → channel 'deploy-prod'", delayAfter: 200 },
      { kind: "output",  text: "  reason: matched deny pattern '*-prod' in [send].channels.deny" },
      { kind: "output",  text: "  fix:    ~/.zad/projects/my-project/services/discord/permissions.toml", delayAfter: 700 },
      { kind: "comment", text: "# Agent proposes a change. You sign. Load-time verify fails closed." },
      { kind: "prompt",  text: "zad discord permissions add --function send \\" },
      { kind: "prompt",  text: "    --target channel --list deny --local 'deploy-*'" },
      { kind: "output",  text: "→ wrote permissions.toml.pending  (run `permissions diff` to inspect)", delayAfter: 600 },
      { kind: "prompt",  text: "zad discord permissions commit --local" },
      { kind: "output",  text: "🔐 keychain unlocked, signed with Ed25519 key, replaced live file." },
    ],
  },
  {
    id: "library",
    label: "Library",
    prompt: "~/my-rust-app $ ",
    lines: [
      { kind: "comment", text: "# Embed zad as a typed library — no subprocess, no MCP server." },
      { kind: "prompt",  text: "cargo add zad" },
      { kind: "output",  text: "    Updating crates.io index" },
      { kind: "output",  text: "      Adding zad v0.6 to dependencies", delayAfter: 500 },
      { kind: "comment", text: "# `SendRequest::new` validates length, attachments, empty payloads" },
      { kind: "comment", text: "# at construction — wrong-shape calls error before any network I/O." },
      { kind: "prompt",  text: "cargo run" },
      { kind: "output",  text: "→ sent message 1336124488907980800", delayAfter: 800 },
    ],
  },
];
