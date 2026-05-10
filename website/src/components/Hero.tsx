import { useState } from "react";
import Terminal from "./terminal";
import { terminalDemos } from "../data/terminalDemos";
import { version, cargo } from "../data/sourceData";
import { cratesCliUrl } from "../seo/siteConfig";

export default function Hero() {
  const [copied, setCopied] = useState(false);

  const copy = () => {
    void navigator.clipboard.writeText("cargo install zad-cli");
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <section className="relative overflow-hidden pt-32 pb-20 md:pt-44 md:pb-28">
      {/* Background glow + dot grid */}
      <div className="bg-grid pointer-events-none absolute inset-0 opacity-60" />
      <div className="pointer-events-none absolute top-0 left-1/2 h-[600px] w-[800px] -translate-x-1/2 rounded-full bg-accent/10 blur-3xl" />

      <div className="relative mx-auto max-w-6xl px-6 text-center">
        <a
          href={cratesCliUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="mb-6 inline-block rounded-full border border-border bg-surface-alt px-4 py-1.5 text-xs text-text-secondary transition-colors hover:border-accent/50 hover:text-text-primary"
        >
          v{version} — on crates.io as <span className="text-accent">zad-cli</span>
        </a>

        <h1 className="mx-auto max-w-4xl text-4xl leading-tight font-extrabold tracking-tight text-text-primary md:text-6xl md:leading-tight">
          Connect your AI agents to{" "}
          <span className="bg-gradient-to-r from-accent to-accent-light bg-clip-text text-transparent">
            real services
          </span>{" "}
          — without an MCP server
        </h1>

        <p className="mx-auto mt-6 max-w-2xl text-lg text-text-secondary md:text-xl">
          One Rust binary, one TOML config, signed permissions. Wire Claude,
          Cursor, or any agent to Discord, Slack, Telegram, Google Calendar,
          Spotify, YouTube Music, and 1Password — and keep tight scopes on
          what they're allowed to do.
        </p>

        <Terminal tabs={terminalDemos} className="mx-auto mt-12 max-w-2xl" />

        <div className="mt-10 flex flex-col items-center gap-4 sm:flex-row sm:justify-center">
          <a
            href="#get-started"
            className="rounded-lg bg-accent px-6 py-3 text-sm font-semibold text-surface shadow-lg shadow-accent/25 transition-colors hover:bg-accent-light"
          >
            Get started
          </a>
          <code className="relative rounded-lg border border-border bg-surface-alt py-3 pr-10 pl-5 text-sm text-text-secondary">
            cargo install zad-cli
            <button
              onClick={copy}
              className="absolute top-1/2 right-2 -translate-y-1/2 cursor-pointer p-1 text-text-secondary transition-colors hover:text-text-primary"
              aria-label="Copy install command"
            >
              {copied ? (
                <svg xmlns="http://www.w3.org/2000/svg" className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="m4.5 12.75 6 6 9-13.5" />
                </svg>
              ) : (
                <svg xmlns="http://www.w3.org/2000/svg" className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 17.25v3.375c0 .621-.504 1.125-1.125 1.125h-9.75a1.125 1.125 0 0 1-1.125-1.125V7.875c0-.621.504-1.125 1.125-1.125H6.75a9.06 9.06 0 0 1 1.5.124m7.5 10.376h3.375c.621 0 1.125-.504 1.125-1.125V11.25c0-4.46-3.243-8.161-7.5-8.876a9.06 9.06 0 0 0-1.5-.124H9.375c-.621 0-1.125.504-1.125 1.125v3.5m7.5 10.375H9.375a1.125 1.125 0 0 1-1.125-1.125v-9.25m12 6.625v-1.875a3.375 3.375 0 0 0-3.375-3.375h-1.5a1.125 1.125 0 0 1-1.125-1.125v-1.5a3.375 3.375 0 0 0-3.375-3.375H9.75" />
                </svg>
              )}
            </button>
          </code>
        </div>

        <p className="mt-6 text-xs text-text-dim">
          Rust {cargo.rustVersion}+ · {cargo.license} licence · macOS keychain, Linux Secret Service, Windows Credential Manager
        </p>
      </div>
    </section>
  );
}
