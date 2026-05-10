import { Link } from "react-router-dom";

const permissionsSnippet = `# ~/.zad/projects/my-project/services/discord/permissions.toml
[content]
deny_words    = ["password", "api_key", "secret"]
deny_patterns = ["(?i)bearer\\\\s+[a-z0-9]+"]
max_length    = 1500          # narrows Discord's 2000 hard cap

[time]
days    = ["mon", "tue", "wed", "thu", "fri"]
windows = ["09:00-18:00"]     # UTC

[send]
channels.allow = ["general", "bot-*", "team/*"]
channels.deny  = ["*admin*", "deploy-prod", "mod-*"]
users.allow    = ["alice", "bob"]
`;

const layers = [
  {
    title: "Scope",
    body: "Coarse on/off per family of operations (e.g. messages.send, gateway.listen). Enforced before any network call.",
    accent: "border-accent/60",
  },
  {
    title: "Global permissions",
    body: "Baseline policy in ~/.zad/services/<svc>/permissions.toml. Apply to every project.",
    accent: "border-allow/50",
  },
  {
    title: "Project permissions",
    body: "~/.zad/projects/<slug>/services/<svc>/permissions.toml. Can only narrow the global rule, never loosen it.",
    accent: "border-allow/50",
  },
  {
    title: "Signature",
    body: "Both files are signed with an Ed25519 key in your OS keychain. Load-time verify fails closed — an agent with FS access can't silently widen.",
    accent: "border-deny/50",
  },
];

export default function Permissions() {
  return (
    <section id="permissions" className="border-t border-border py-20 md:py-28">
      <div className="mx-auto max-w-6xl px-6">
        <div className="grid gap-12 lg:grid-cols-2 lg:items-start">
          <div>
            <span className="text-xs font-semibold tracking-widest text-accent uppercase">
              The trust model
            </span>
            <h2 className="mt-3 text-3xl font-bold text-text-primary md:text-4xl">
              Permissions that <span className="text-accent">survive contact</span> with an agent
            </h2>
            <p className="mt-4 text-text-secondary">
              MCP servers gate behind a UI checkbox at install time. zad gates
              every call against four layers, in order. The strictest wins;
              an empty allow list is "no positive constraint", not "deny all";
              deny always beats allow.
            </p>

            <div className="mt-8 space-y-3">
              {layers.map((l, i) => (
                <div
                  key={l.title}
                  className={`flex gap-4 rounded-lg border ${l.accent} bg-surface-alt p-4`}
                >
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-border bg-surface text-sm font-mono text-text-dim">
                    {i + 1}
                  </div>
                  <div>
                    <h3 className="font-semibold text-text-primary">{l.title}</h3>
                    <p className="mt-1 text-sm text-text-secondary">{l.body}</p>
                  </div>
                </div>
              ))}
            </div>

            <p className="mt-8 text-sm text-text-dim">
              Agents propose changes via a staged{" "}
              <code className="rounded bg-surface-alt px-1.5 py-0.5 text-text-secondary">.pending</code>{" "}
              file; you sign and{" "}
              <code className="rounded bg-surface-alt px-1.5 py-0.5 text-text-secondary">commit</code>{" "}
              to replace the live policy. Read the full model in the{" "}
              <Link to="/docs/permissions" className="text-accent hover:text-accent-light underline">
                Permissions doc
              </Link>
              .
            </p>
          </div>

          <div className="relative">
            <div className="pointer-events-none absolute -inset-3 rounded-2xl bg-accent/10 blur-2xl" />
            <div className="relative overflow-hidden rounded-xl border border-border bg-surface-alt shadow-2xl">
              <div className="flex items-center justify-between border-b border-border px-4 py-2 text-xs">
                <span className="font-mono text-text-dim">permissions.toml</span>
                <span className="rounded bg-deny/10 px-2 py-0.5 font-mono text-deny">
                  signed · load-time verified
                </span>
              </div>
              <pre className="overflow-x-auto px-5 py-4 text-[12.5px] leading-6 text-text-secondary">
                <code>{permissionsSnippet}</code>
              </pre>
              <div className="border-t border-border bg-surface px-5 py-4 text-xs text-text-dim">
                <div className="font-mono text-deny">
                  $ zad discord permissions check --function send --channel deploy-prod
                </div>
                <div className="mt-1 font-mono">
                  PermissionDenied: matched deny pattern <span className="text-text-secondary">'deploy-prod'</span> in [send].channels.deny
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
