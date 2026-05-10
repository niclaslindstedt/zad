// Post-build pass that runs after `vite build`.
//
// Per OSS_SPEC §11.3 ("pre-rendered metadata for single-page apps"),
// every public route needs its own <title>, <meta name="description">,
// canonical link, Open Graph tags, Twitter card, and JSON-LD before
// crawlers see the page — they don't run our React code.
//
// This script:
//   1. Loads the SEO config (single source of truth, §11.3).
//   2. Reads dist/index.html (the SPA shell vite emitted).
//   3. For each public route, writes dist/<route>/index.html with
//      route-specific <head> values spliced in. Body stays untouched.
//   4. Writes dist/404.html as a copy of dist/index.html so SPA
//      fallback hosting renders the homepage on unknown URLs.
//   5. Regenerates dist/sitemap.xml from the same route table, with
//      <lastmod> values pulled from real source data (file mtimes for
//      docs/man pages, latest commit otherwise).

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const repoRoot = path.resolve(
  path.dirname(new URL(import.meta.url).pathname),
  "..",
  "..",
);
const distDir = path.join(repoRoot, "website", "dist");
const seoConfigPath = path.join(repoRoot, "website", "src", "seo", "siteConfig.ts");

function parseStringConsts(src) {
  // Very small TypeScript reader: pulls out
  //   export const NAME = "value";
  // entries. We don't need a real parser for a hand-curated file.
  const out = {};
  const re = /export const (\w+)\s*=\s*"([^"]*)"/g;
  for (const m of src.matchAll(re)) out[m[1]] = m[2];
  return out;
}

function parseRoutes(src) {
  // Pull `export const routes: RouteSeo[] = [ { path, title, description,
  // schemaType }, ... ]` out of the TS file without involving a TS toolchain.
  const start = src.indexOf("export const routes");
  if (start === -1) throw new Error("siteConfig.ts: missing `export const routes`");
  const open = src.indexOf("[", start);
  const close = src.indexOf("];", open);
  if (open === -1 || close === -1) throw new Error("siteConfig.ts: malformed routes array");
  const body = src.slice(open + 1, close);
  // Carve `body` into top-level `{ ... }` slabs. We can't use a single
  // greedy regex because `}` inside a template literal (e.g. `${x}`) would
  // terminate the match early. Instead, walk the string tracking quote
  // state and bracket depth.
  function scanObjects(text) {
    const out = [];
    let i = 0;
    while (i < text.length) {
      while (i < text.length && text[i] !== "{") i++;
      if (i >= text.length) break;
      let depth = 0;
      const start = i;
      let inStr = null; // '"' | "'" | '`' | null
      for (; i < text.length; i++) {
        const ch = text[i];
        if (inStr) {
          if (ch === "\\") {
            i++;
            continue;
          }
          if (ch === inStr) inStr = null;
          continue;
        }
        if (ch === '"' || ch === "'" || ch === "`") {
          inStr = ch;
          continue;
        }
        if (ch === "{") depth++;
        else if (ch === "}") {
          depth--;
          if (depth === 0) {
            out.push(text.slice(start + 1, i));
            i++;
            break;
          }
        }
      }
    }
    return out;
  }

  const routes = [];
  // The mini-parser handles three value shapes per key:
  //   key: "literal"      → use the literal
  //   key: `template ${x}` → resolve ${x} against the string consts
  //   key: identifier      → look identifier up in the string consts
  // Anything else (numbers, function calls, computed expressions) would
  // need a real TS parser; we keep the supported shapes deliberately small.
  const consts = parseStringConsts(src);
  for (const inner of scanObjects(body)) {
    const pick = (k) => {
      const tpl = inner.match(new RegExp(`${k}\\s*:\\s*\`([^\`]*)\``));
      if (tpl) return tpl[1];
      const lit = inner.match(new RegExp(`${k}\\s*:\\s*"([^"]*)"`));
      if (lit) return lit[1];
      const ident = inner.match(new RegExp(`${k}\\s*:\\s*([A-Za-z_][A-Za-z0-9_]*)`));
      if (ident) return consts[ident[1]];
      return undefined;
    };
    routes.push({
      path: pick("path"),
      title: pick("title"),
      description: pick("description"),
      schemaType: pick("schemaType"),
    });
  }
  if (routes.length === 0) throw new Error("siteConfig.ts: routes array is empty");
  // Resolve template literals like `${siteName} — ${siteTagline}` against
  // the simple string consts we already parsed.
  for (const r of routes) {
    for (const key of ["title", "description"]) {
      if (typeof r[key] === "string") {
        r[key] = r[key].replace(/\$\{(\w+)\}/g, (_, name) => consts[name] ?? "");
      }
    }
  }
  return { consts, routes };
}

const seoSrc = fs.readFileSync(seoConfigPath, "utf8");
const { consts, routes } = parseRoutes(seoSrc);

const siteUrl = consts.siteUrl;
const siteName = consts.siteName;
const siteDescription = consts.siteDescription;
const ogImage = `${siteUrl}og-default.png`;
const ogImageWidth = 1200;
const ogImageHeight = 630;

if (!siteUrl || !siteName) {
  throw new Error("siteConfig.ts is missing siteUrl or siteName");
}

function escape(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function jsonLdFor(route) {
  const url = route.path === "/" ? siteUrl : `${siteUrl}${route.path.replace(/^\//, "")}`;
  if (route.schemaType === "SoftwareApplication") {
    return {
      "@context": "https://schema.org",
      "@type": "SoftwareApplication",
      "@id": siteUrl,
      name: siteName,
      description: siteDescription,
      applicationCategory: "DeveloperApplication",
      operatingSystem: "Linux, macOS, Windows",
      url: siteUrl,
      image: ogImage,
      offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
    };
  }
  return {
    "@context": "https://schema.org",
    "@type": route.schemaType,
    "@id": url,
    name: route.title,
    description: route.description,
    url,
    isPartOf: { "@type": "WebSite", "@id": siteUrl, name: siteName },
  };
}

function renderHead(route) {
  const url = route.path === "/" ? siteUrl : `${siteUrl}${route.path.replace(/^\//, "")}`;
  const title = route.title;
  const description = route.description;
  const jsonLd = JSON.stringify(jsonLdFor(route));
  return `    <title>${escape(title)}</title>
    <meta name="description" content="${escape(description)}" />
    <link rel="canonical" href="${escape(url)}" />
    <meta name="robots" content="index,follow,max-image-preview:large" />
    <meta property="og:site_name" content="${escape(siteName)}" />
    <meta property="og:type" content="website" />
    <meta property="og:title" content="${escape(title)}" />
    <meta property="og:description" content="${escape(description)}" />
    <meta property="og:url" content="${escape(url)}" />
    <meta property="og:image" content="${escape(ogImage)}" />
    <meta property="og:image:width" content="${ogImageWidth}" />
    <meta property="og:image:height" content="${ogImageHeight}" />
    <meta property="og:image:alt" content="${escape(`${siteName} — ${description}`)}" />
    <meta name="twitter:card" content="summary_large_image" />
    <meta name="twitter:title" content="${escape(title)}" />
    <meta name="twitter:description" content="${escape(description)}" />
    <meta name="twitter:image" content="${escape(ogImage)}" />
    <link rel="sitemap" type="application/xml" href="/sitemap.xml" />
    <script type="application/ld+json">${jsonLd}</script>`;
}

function spliceHead(shellHtml, route) {
  // Replace everything between <head>'s charset/viewport block and </head>.
  // We anchor the rewrite on the canonical link so we don't disturb the
  // bundle's <link>/<script> tags that vite injects above us.
  const headOpen = shellHtml.indexOf("<head>");
  const headClose = shellHtml.indexOf("</head>");
  if (headOpen === -1 || headClose === -1) {
    throw new Error("dist/index.html has no <head>...</head> — vite output broken");
  }
  const headInner = shellHtml.slice(headOpen + "<head>".length, headClose);

  // Strip the route-specific tags we own (title/description/canonical/og*/
  // twitter*/robots/sitemap link/JSON-LD) — keep everything else (favicon,
  // viewport, charset, vite asset tags).
  const ownedTagPatterns = [
    /<title>[\s\S]*?<\/title>\s*/g,
    /<meta\s+name="description"[^>]*>\s*/g,
    /<link\s+rel="canonical"[^>]*>\s*/g,
    /<meta\s+name="robots"[^>]*>\s*/g,
    /<meta\s+property="og:[^"]+"[^>]*>\s*/g,
    /<meta\s+name="twitter:[^"]+"[^>]*>\s*/g,
    /<link\s+rel="sitemap"[^>]*>\s*/g,
    /<script\s+type="application\/ld\+json">[\s\S]*?<\/script>\s*/g,
  ];
  let stripped = headInner;
  for (const re of ownedTagPatterns) stripped = stripped.replace(re, "");

  return (
    shellHtml.slice(0, headOpen + "<head>".length) +
    "\n" + stripped.trimEnd() + "\n" + renderHead(route) + "\n  " +
    shellHtml.slice(headClose)
  );
}

const shell = fs.readFileSync(path.join(distDir, "index.html"), "utf8");

// 1. Per-route HTML (homepage overwrites dist/index.html in place).
for (const route of routes) {
  const html = spliceHead(shell, route);
  if (route.path === "/") {
    fs.writeFileSync(path.join(distDir, "index.html"), html);
    console.log("rewrote dist/index.html");
    continue;
  }
  const dir = path.join(distDir, route.path.replace(/^\//, ""));
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "index.html"), html);
  console.log("wrote", path.posix.join("dist", route.path.replace(/^\//, ""), "index.html"));
}

// 2. SPA fallback so unknown URLs render the homepage shell.
fs.copyFileSync(path.join(distDir, "index.html"), path.join(distDir, "404.html"));
console.log("wrote dist/404.html");

// 3. Sitemap. Listed routes plus every doc/man slug; <lastmod> uses the
// most recent commit touching the source file (falls back to file mtime
// when git is unavailable).
function lastmodFor(filePath) {
  try {
    const stamp = execFileSync("git", ["log", "-1", "--format=%cs", "--", filePath], {
      cwd: repoRoot,
      encoding: "utf8",
    }).trim();
    if (stamp) return stamp;
  } catch {
    // git missing or path untracked — fall through to mtime.
  }
  try {
    const st = fs.statSync(filePath);
    return st.mtime.toISOString().slice(0, 10);
  } catch {
    return new Date().toISOString().slice(0, 10);
  }
}

const docsDir = path.join(repoRoot, "docs");
const manDir = path.join(repoRoot, "man");
const docsSlugs = fs
  .readdirSync(docsDir)
  .filter((f) => f.endsWith(".md"))
  .map((f) => f.replace(/\.md$/, ""))
  .sort();
const manSlugs = fs
  .readdirSync(manDir)
  .filter((f) => f.endsWith(".md"))
  .map((f) => f.replace(/\.md$/, ""))
  .sort();

const allUrls = [
  { loc: siteUrl, lastmod: lastmodFor(path.join(repoRoot, "README.md")) },
  ...docsSlugs.map((s) => ({
    loc: `${siteUrl}docs/${s}`,
    lastmod: lastmodFor(path.join(docsDir, `${s}.md`)),
  })),
  ...manSlugs.map((s) => ({
    loc: `${siteUrl}manual/${s}`,
    lastmod: lastmodFor(path.join(manDir, `${s}.md`)),
  })),
];

const sitemap =
  `<?xml version="1.0" encoding="UTF-8"?>\n` +
  `<urlset xmlns="http://www.sitemap.org/schemas/sitemap/0.9">\n` +
  allUrls
    .map(
      (u) =>
        `  <url>\n    <loc>${escape(u.loc)}</loc>\n    <lastmod>${u.lastmod}</lastmod>\n  </url>`,
    )
    .join("\n") +
  `\n</urlset>\n`;

fs.writeFileSync(path.join(distDir, "sitemap.xml"), sitemap);
console.log(`wrote dist/sitemap.xml (${allUrls.length} URLs)`);

// 4. Smoke check (per OSS_SPEC §11.3 CI verification): bail out if the
// homepage is missing any required SEO element. This catches future
// regressions in the splicer.
const home = fs.readFileSync(path.join(distDir, "index.html"), "utf8");
const required = [
  ["<title>", "homepage <title>"],
  [`<link rel="canonical"`, "canonical link"],
  [`application/ld+json`, "JSON-LD block"],
  [`og:image`, "og:image meta"],
  [`twitter:card`, "twitter card"],
  [`name="description"`, "meta description"],
];
const missing = required.filter(([needle]) => !home.includes(needle));
if (missing.length) {
  throw new Error(
    `Post-build SEO check failed — missing: ${missing.map(([, label]) => label).join(", ")}`,
  );
}
console.log("SEO smoke check OK");

// Make tooling that imports siteConfig with the file URL helper happy
// even though we don't use the import itself.
void pathToFileURL;
