import { Link } from "react-router-dom";
import { quickStart } from "../data/sourceData";
import { repoUrl } from "../seo/siteConfig";

const installOptions = [
  {
    label: "curl",
    title: "Install script",
    body: "Drops a pre-built binary into the first writable directory on your PATH (preferring ~/.local/bin).",
    code: "curl -fsSL https://raw.githubusercontent.com/niclaslindstedt/zad/main/scripts/install.sh | sh",
  },
  {
    label: "cargo",
    title: "From crates.io",
    body: "Builds the same binary from source. Installs an executable named `zad`.",
    code: "cargo install zad-cli",
  },
  {
    label: "source",
    title: "Local checkout",
    body: "Clone the repo and install the binary against your working tree.",
    code: "git clone https://github.com/niclaslindstedt/zad.git\ncd zad && cargo install --path crates/zad-cli",
  },
];

export default function GetStarted() {
  return (
    <section id="get-started" className="border-t border-border py-20 md:py-28">
      <div className="mx-auto max-w-6xl px-6">
        <div className="text-center">
          <h2 className="text-3xl font-bold text-text-primary md:text-4xl">Get started in 60 seconds</h2>
          <p className="mx-auto mt-4 max-w-2xl text-text-secondary">
            Pick an install path, register one bot, opt in per project. zad takes
            care of the OS keychain, the OAuth dance, and the directory cache.
          </p>
        </div>

        <div className="mt-12 grid gap-6 md:grid-cols-3">
          {installOptions.map((opt) => (
            <div
              key={opt.label}
              className="flex flex-col rounded-xl border border-border bg-surface-alt p-6"
            >
              <div className="mb-2 text-xs font-semibold tracking-widest text-accent uppercase">
                {opt.label}
              </div>
              <h3 className="text-lg font-semibold text-text-primary">{opt.title}</h3>
              <p className="mt-2 text-sm text-text-secondary">{opt.body}</p>
              <pre className="mt-4 overflow-x-auto rounded-md border border-border bg-surface px-3 py-2 text-xs leading-6 text-text-secondary">
                <code>{opt.code}</code>
              </pre>
            </div>
          ))}
        </div>

        <div className="mt-14 overflow-hidden rounded-xl border border-border bg-surface-alt shadow-xl">
          <div className="flex items-center justify-between border-b border-border px-4 py-2 text-xs">
            <span className="font-mono text-text-dim">README.md → ## Quick start</span>
            <span className="font-mono text-text-dim">extracted from source</span>
          </div>
          <pre className="overflow-x-auto px-5 py-5 text-[12.5px] leading-6 text-text-secondary">
            <code>{quickStart}</code>
          </pre>
        </div>

        <div className="mt-10 flex flex-wrap items-center justify-center gap-4">
          <Link
            to="/docs/getting-started"
            className="rounded-lg bg-accent px-6 py-3 text-sm font-semibold text-surface shadow-lg shadow-accent/25 transition-colors hover:bg-accent-light"
          >
            Read the getting-started guide
          </Link>
          <a
            href={repoUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="rounded-lg border border-border px-6 py-3 text-sm text-text-secondary transition-colors hover:border-accent hover:text-text-primary"
          >
            Browse the source on GitHub
          </a>
        </div>
      </div>
    </section>
  );
}
