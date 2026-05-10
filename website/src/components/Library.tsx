import { librarySnippet, version } from "../data/sourceData";
import { cratesLibUrl } from "../seo/siteConfig";

const cargoToml = `[dependencies]
zad = "${version.split(".").slice(0, 2).join(".")}"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
`;

export default function Library() {
  return (
    <section id="library" className="border-t border-border py-20 md:py-28">
      <div className="mx-auto max-w-6xl px-6">
        <div className="grid gap-12 lg:grid-cols-2 lg:items-start">
          <div>
            <span className="text-xs font-semibold tracking-widest text-accent uppercase">
              Library, not just a CLI
            </span>
            <h2 className="mt-3 text-3xl font-bold text-text-primary md:text-4xl">
              Embed it in Rust with a <span className="text-accent">typed facade</span>
            </h2>
            <p className="mt-4 text-text-secondary">
              The <code className="rounded bg-surface-alt px-1.5 py-0.5 text-text-primary">zad</code> crate
              is the same code path the CLI uses, exposed as a typed Rust API.
              Newtypes for snowflakes, validation at construction, and a
              dedicated error type — no shelling out, no JSON wrangling, no
              MCP server in front of you.
            </p>

            <ul className="mt-6 space-y-3 text-sm">
              <li className="flex gap-3 text-text-secondary">
                <span className="text-allow">✓</span>
                <span>
                  <code className="rounded bg-surface-alt px-1.5 py-0.5 text-text-primary">SendRequest::new</code>{" "}
                  validates body length, attachments, and empty-payload rules
                  before any network I/O.
                </span>
              </li>
              <li className="flex gap-3 text-text-secondary">
                <span className="text-allow">✓</span>
                <span>
                  <code className="rounded bg-surface-alt px-1.5 py-0.5 text-text-primary">ChannelId(u64)</code>,{" "}
                  <code className="rounded bg-surface-alt px-1.5 py-0.5 text-text-primary">UserId(u64)</code>,{" "}
                  <code className="rounded bg-surface-alt px-1.5 py-0.5 text-text-primary">MessageId(u64)</code>{" "}
                  newtypes — pass the wrong kind of snowflake and the compiler stops you.
                </span>
              </li>
              <li className="flex gap-3 text-text-secondary">
                <span className="text-allow">✓</span>
                <span>
                  <code className="rounded bg-surface-alt px-1.5 py-0.5 text-text-primary">Discord::with_paths(...)</code>{" "}
                  reads zero environment variables — the recommended entry
                  point for production servers, multi-tenant code, and
                  deterministic tests.
                </span>
              </li>
              <li className="flex gap-3 text-text-secondary">
                <span className="text-allow">✓</span>
                <span>
                  Permissions enforced inside the library, before the
                  HTTP/WebSocket call. The CLI doesn't get a stricter check
                  than your code does.
                </span>
              </li>
            </ul>

            <a
              href={cratesLibUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="mt-8 inline-flex items-center gap-2 rounded-lg border border-border bg-surface-alt px-4 py-2 text-sm text-text-secondary transition-colors hover:border-accent hover:text-text-primary"
            >
              View <code>zad</code> on crates.io →
            </a>
          </div>

          <div className="space-y-4">
            <div className="overflow-hidden rounded-xl border border-border bg-surface-alt shadow-xl">
              <div className="flex items-center justify-between border-b border-border px-4 py-2 text-xs">
                <span className="font-mono text-text-dim">Cargo.toml</span>
                <span className="font-mono text-text-dim">add to your project</span>
              </div>
              <pre className="overflow-x-auto px-5 py-4 text-[12.5px] leading-6 text-text-secondary">
                <code>{cargoToml}</code>
              </pre>
            </div>

            <div className="overflow-hidden rounded-xl border border-border bg-surface-alt shadow-xl">
              <div className="flex items-center justify-between border-b border-border px-4 py-2 text-xs">
                <span className="font-mono text-text-dim">src/main.rs</span>
                <span className="font-mono text-text-dim">runnable example</span>
              </div>
              <pre className="overflow-x-auto px-5 py-4 text-[12.5px] leading-6 text-text-secondary">
                <code>{librarySnippet}</code>
              </pre>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
