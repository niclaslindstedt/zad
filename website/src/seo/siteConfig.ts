// Single source of truth for SEO copy (per OSS_SPEC §11.3). Imported by
// runtime client code and by the post-build SEO splicer.

export const siteUrl = "https://zad.niclaslindstedt.se/";
export const siteName = "zad";
export const siteTagline =
  "Connect AI agents to external services without an MCP server";
export const siteDescription =
  "A Rust library and CLI that connects AI agents to Discord, Slack, Telegram, Google Calendar, Spotify, YouTube Music, and 1Password via signed, scoped TOML configs — no MCP server required.";

export const ogImage = `${siteUrl}og-default.png`;
export const ogImageWidth = 1200;
export const ogImageHeight = 630;

export const repoUrl = "https://github.com/niclaslindstedt/zad";
export const cratesCliUrl = "https://crates.io/crates/zad-cli";
export const cratesLibUrl = "https://crates.io/crates/zad";

export interface RouteSeo {
  path: string;
  title: string;
  description: string;
  schemaType: "SoftwareApplication" | "TechArticle" | "CollectionPage";
}

// Static routes the SPA serves. Sub-pages for individual docs/man pages
// fall back to the homepage shell — they are still reachable, but their
// per-page <title> is set client-side. The post-build splicer rewrites
// the <head> for these enumerated routes only (a reasonable trade-off
// for a small landing site).
export const routes: RouteSeo[] = [
  {
    path: "/",
    title: `${siteName} — ${siteTagline}`,
    description: siteDescription,
    schemaType: "SoftwareApplication",
  },
  {
    path: "/docs",
    title: `Documentation — ${siteName}`,
    description: `Hosted documentation for ${siteName}: getting started, configuration, architecture, services, and the permissions trust model.`,
    schemaType: "CollectionPage",
  },
  {
    path: "/manual",
    title: `Manual — ${siteName}`,
    description: `Per-command reference for ${siteName}: every subcommand and flag the CLI accepts.`,
    schemaType: "CollectionPage",
  },
];
