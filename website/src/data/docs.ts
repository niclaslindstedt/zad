// Hosted-docs index. Each entry imports a `docs/<slug>.md` file via Vite's
// `?raw` loader so the rendered text always matches what's checked into
// the repo — no second copy to keep in sync.

import architecture from "../../../docs/architecture.md?raw";
import configuration from "../../../docs/configuration.md?raw";
import gettingStarted from "../../../docs/getting-started.md?raw";
import permissions from "../../../docs/permissions.md?raw";
import services from "../../../docs/services.md?raw";
import troubleshooting from "../../../docs/troubleshooting.md?raw";

import { docsList } from "./sourceData";

export interface DocPage {
  slug: string;
  title: string;
  content: string;
}

export const docs: DocPage[] = [
  { slug: "getting-started", title: "Getting started", content: gettingStarted },
  { slug: "configuration",   title: "Configuration",   content: configuration },
  { slug: "services",        title: "Services",        content: services },
  { slug: "permissions",     title: "Permissions",     content: permissions },
  { slug: "architecture",    title: "Architecture",    content: architecture },
  { slug: "troubleshooting", title: "Troubleshooting", content: troubleshooting },
];

// Drift guard: every file under docs/ must appear in this index, and
// every entry must point at a file that exists. Otherwise the docs
// section silently drops or breaks pages.
const have = new Set(docsList);
const declared = new Set(docs.map((d) => d.slug));
const missing = docsList.filter((s) => !declared.has(s));
const extra = docs.map((d) => d.slug).filter((s) => !have.has(s));
if (missing.length || extra.length) {
  throw new Error(
    `docs.ts is out of sync with docs/ on disk. ` +
      `Missing index entry for: ${missing.join(", ") || "(none)"}. ` +
      `Stale entries (no source file): ${extra.join(", ") || "(none)"}. ` +
      `Update website/src/data/docs.ts to match.`,
  );
}

export function getDocBySlug(slug: string): DocPage | undefined {
  return docs.find((d) => d.slug === slug);
}
