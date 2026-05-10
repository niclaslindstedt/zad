// Per-command manual pages. Each entry imports `man/<slug>.md` via Vite's
// `?raw` loader, so the rendered manual stays bit-for-bit identical to the
// version embedded in the binary.

import main from "../../../man/main.md?raw";
import service from "../../../man/service.md?raw";
import discord from "../../../man/discord.md?raw";
import slack from "../../../man/slack.md?raw";
import telegram from "../../../man/telegram.md?raw";
import gcal from "../../../man/gcal.md?raw";
import spotify from "../../../man/spotify.md?raw";
import ymusic from "../../../man/ymusic.md?raw";
import onepass from "../../../man/1pass.md?raw";
import commands from "../../../man/commands.md?raw";
import docs from "../../../man/docs.md?raw";
import man from "../../../man/man.md?raw";
import signing from "../../../man/signing.md?raw";

import { manpagesList } from "./sourceData";

export interface ManPage {
  slug: string;
  title: string;
  content: string;
}

export const manpages: ManPage[] = [
  { slug: "main",     title: "zad",              content: main },
  { slug: "service",  title: "zad service",      content: service },
  { slug: "1pass",    title: "zad 1pass",        content: onepass },
  { slug: "discord",  title: "zad discord",      content: discord },
  { slug: "gcal",     title: "zad gcal",         content: gcal },
  { slug: "slack",    title: "zad slack",        content: slack },
  { slug: "spotify",  title: "zad spotify",      content: spotify },
  { slug: "telegram", title: "zad telegram",     content: telegram },
  { slug: "ymusic",   title: "zad ymusic",       content: ymusic },
  { slug: "commands", title: "zad commands",     content: commands },
  { slug: "docs",     title: "zad docs",         content: docs },
  { slug: "man",      title: "zad man",          content: man },
  { slug: "signing",  title: "Signing & trust",  content: signing },
];

const have = new Set(manpagesList);
const declared = new Set(manpages.map((m) => m.slug));
const missing = manpagesList.filter((s) => !declared.has(s));
const extra = manpages.map((m) => m.slug).filter((s) => !have.has(s));
if (missing.length || extra.length) {
  throw new Error(
    `manpages.ts is out of sync with man/ on disk. ` +
      `Missing index entry for: ${missing.join(", ") || "(none)"}. ` +
      `Stale entries (no source file): ${extra.join(", ") || "(none)"}. ` +
      `Update website/src/data/manpages.ts to match.`,
  );
}

export function getManpageBySlug(slug: string): ManPage | undefined {
  return manpages.find((m) => m.slug === slug);
}
