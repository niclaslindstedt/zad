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
//   3. For each public route — top-level routes from siteConfig plus
//      every `/docs/<slug>` and `/manual/<slug>` derived from the .md
//      files in `docs/` and `man/` — writes dist/<route>/index.html
//      with route-specific <head> values spliced in. Body stays
//      untouched.
//   4. Writes dist/404.html as a copy of dist/index.html so SPA
//      fallback hosting renders the homepage on unknown URLs.
//   5. Regenerates dist/sitemap.xml from the same route table, with
//      <lastmod> values pulled from real source data (file mtimes for
//      docs/man pages, latest commit otherwise) and search-engine
//      hints (<priority>, <changefreq>).

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

function parseStringArray(src, name) {
  // Pulls `export const NAME: string[] = [ "a", "b", ... ];` out of the TS
  // file. Used for siteKeywords. Tolerates trailing commas and newlines
  // between entries.
  const re = new RegExp(`export const ${name}[^=]*=\\s*\\[([\\s\\S]*?)\\];`);
  const m = src.match(re);
  if (!m) return [];
  return [...m[1].matchAll(/"([^"]*)"/g)].map((x) => x[1]);
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
const { consts, routes: topRoutes } = parseRoutes(seoSrc);
const siteKeywords = parseStringArray(seoSrc, "siteKeywords");

const siteUrl = consts.siteUrl;
const siteName = consts.siteName;
const siteDescription = consts.siteDescription;
const author = consts.author || "";
const authorUrl = consts.authorUrl || "";
const language = consts.language || "en";
const themeColor = consts.themeColor || "";
const applicationName = consts.applicationName || siteName;
const repoUrl = consts.repoUrl || "";
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

function readMarkdownMeta(filePath) {
  // Convention: docs/*.md and man/*.md start with
  //   # Title
  //   <blank>
  //   > One-line tagline (optional)
  // Pull the first H1 and the first paragraph (blockquote or otherwise)
  // so the splicer can synthesise per-page <title> / <meta description>
  // without a parallel hand-maintained table. We collect every line of
  // the first paragraph until a blank line so wrapped Markdown produces
  // a complete sentence rather than a truncated fragment.
  const text = fs.readFileSync(filePath, "utf8");
  const lines = text.split(/\r?\n/);
  let title = "";
  const buf = [];
  let inParagraph = false;
  for (const line of lines) {
    if (!title && /^#\s+/.test(line)) {
      title = line.replace(/^#\s+/, "").trim();
      continue;
    }
    if (!title) continue;
    if (line.trim() === "") {
      if (inParagraph) break;
      continue;
    }
    if (line.startsWith("#")) {
      if (inParagraph) break;
      continue;
    }
    inParagraph = true;
    buf.push(line.replace(/^>\s*/, "").trim());
  }
  const tagline = buf
    .join(" ")
    .replace(/\s+/g, " ")
    .replace(/`/g, "")
    .trim();
  return { title, tagline };
}

function deriveDocRoute(slug) {
  const file = path.join(repoRoot, "docs", `${slug}.md`);
  const meta = readMarkdownMeta(file);
  const niceTitle = meta.title || slug.replace(/-/g, " ");
  const description = meta.tagline
    ? `${meta.tagline} — ${siteName} documentation.`
    : `${niceTitle} — ${siteName} documentation. Connect AI agents to external services without an MCP server.`;
  return {
    path: `/docs/${slug}`,
    title: `${niceTitle} — ${siteName} documentation`,
    description: truncate(description, 300),
    schemaType: "TechArticle",
    sourceFile: file,
    breadcrumb: [
      { name: "Home", url: siteUrl },
      { name: "Documentation", url: `${siteUrl}docs` },
      { name: niceTitle, url: `${siteUrl}docs/${slug}` },
    ],
  };
}

function deriveManRoute(slug) {
  const file = path.join(repoRoot, "man", `${slug}.md`);
  const meta = readMarkdownMeta(file);
  const niceTitle = meta.title || `zad ${slug}`;
  const description = meta.tagline
    ? `${meta.tagline} — ${siteName} manual.`
    : `${niceTitle} — ${siteName} CLI manual page. Reference for the ${slug} command.`;
  return {
    path: `/manual/${slug}`,
    title: `${niceTitle} — ${siteName} manual`,
    description: truncate(description, 300),
    schemaType: "TechArticle",
    sourceFile: file,
    breadcrumb: [
      { name: "Home", url: siteUrl },
      { name: "Manual", url: `${siteUrl}manual` },
      { name: niceTitle, url: `${siteUrl}manual/${slug}` },
    ],
  };
}

function truncate(s, n) {
  if (!s) return s;
  if (s.length <= n) return s;
  return s.slice(0, n - 1).trimEnd() + "…";
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

const docRoutes = docsSlugs.map(deriveDocRoute);
const manRoutes = manSlugs.map(deriveManRoute);
const allRoutes = [...topRoutes, ...docRoutes, ...manRoutes];

function jsonLdFor(route) {
  const url = route.path === "/" ? siteUrl : `${siteUrl}${route.path.replace(/^\//, "")}`;

  if (route.schemaType === "SoftwareApplication") {
    // Homepage emits a graph: software + source code + website. This
    // gives crawlers an explicit hint that the package on crates.io and
    // the source on GitHub are the same project as this landing page.
    return {
      "@context": "https://schema.org",
      "@graph": [
        {
          "@type": "SoftwareApplication",
          "@id": `${siteUrl}#software`,
          name: siteName,
          alternateName: "zad-cli",
          description: siteDescription,
          applicationCategory: "DeveloperApplication",
          applicationSubCategory: "CommandLine",
          operatingSystem: "Linux, macOS, Windows",
          url: siteUrl,
          image: ogImage,
          license: "https://opensource.org/licenses/MIT",
          downloadUrl: "https://crates.io/crates/zad-cli",
          softwareHelp: `${siteUrl}docs`,
          programmingLanguage: "Rust",
          keywords: siteKeywords.join(", "),
          offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
          author: author
            ? { "@type": "Person", name: author, url: authorUrl }
            : undefined,
        },
        repoUrl
          ? {
              "@type": "SoftwareSourceCode",
              "@id": `${siteUrl}#source`,
              name: siteName,
              description: `Source code for the ${siteName} Rust library and CLI.`,
              codeRepository: repoUrl,
              programmingLanguage: "Rust",
              url: repoUrl,
              license: "https://opensource.org/licenses/MIT",
            }
          : undefined,
        {
          "@type": "WebSite",
          "@id": `${siteUrl}#website`,
          url: siteUrl,
          name: siteName,
          description: siteDescription,
          inLanguage: language,
          publisher: author
            ? { "@type": "Person", name: author, url: authorUrl }
            : undefined,
        },
      ].filter(Boolean),
    };
  }

  if (route.schemaType === "TechArticle") {
    // Per-doc / per-manpage pages emit TechArticle plus a BreadcrumbList
    // so SERPs render the "Home > Docs > Page" path under the listing.
    const article = {
      "@type": "TechArticle",
      "@id": url,
      headline: route.title,
      name: route.title,
      description: route.description,
      url,
      inLanguage: language,
      isPartOf: { "@type": "WebSite", "@id": `${siteUrl}#website`, name: siteName },
      author: author
        ? { "@type": "Person", name: author, url: authorUrl }
        : undefined,
      mainEntityOfPage: url,
    };
    const breadcrumb = route.breadcrumb && {
      "@type": "BreadcrumbList",
      itemListElement: route.breadcrumb.map((b, i) => ({
        "@type": "ListItem",
        position: i + 1,
        name: b.name,
        item: b.url,
      })),
    };
    return {
      "@context": "https://schema.org",
      "@graph": [article, breadcrumb].filter(Boolean),
    };
  }

  // CollectionPage and anything else.
  return {
    "@context": "https://schema.org",
    "@type": route.schemaType || "WebPage",
    "@id": url,
    name: route.title,
    description: route.description,
    url,
    inLanguage: language,
    isPartOf: { "@type": "WebSite", "@id": `${siteUrl}#website`, name: siteName },
  };
}

function renderHead(route) {
  const url = route.path === "/" ? siteUrl : `${siteUrl}${route.path.replace(/^\//, "")}`;
  const title = route.title;
  const description = route.description;
  const jsonLd = JSON.stringify(jsonLdFor(route));
  const keywordsAttr = siteKeywords.length
    ? `\n    <meta name="keywords" content="${escape(siteKeywords.join(", "))}" />`
    : "";
  const themeAttr = themeColor
    ? `\n    <meta name="theme-color" content="${escape(themeColor)}" />`
    : "";
  const appNameAttr = applicationName
    ? `\n    <meta name="application-name" content="${escape(applicationName)}" />`
    : "";
  const authorAttr = author
    ? `\n    <meta name="author" content="${escape(author)}" />`
    : "";
  const authorLink = authorUrl
    ? `\n    <link rel="author" href="${escape(authorUrl)}" />`
    : "";
  return `    <title>${escape(title)}</title>
    <meta name="description" content="${escape(description)}" />${keywordsAttr}${themeAttr}${appNameAttr}${authorAttr}${authorLink}
    <link rel="canonical" href="${escape(url)}" />
    <meta name="robots" content="index,follow,max-image-preview:large" />
    <meta name="googlebot" content="index,follow,max-image-preview:large,max-snippet:-1" />
    <meta name="bingbot" content="index,follow,max-image-preview:large,max-snippet:-1" />
    <meta property="og:site_name" content="${escape(siteName)}" />
    <meta property="og:locale" content="en_US" />
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
    <meta name="twitter:image:alt" content="${escape(`${siteName} — ${description}`)}" />
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

  // Strip the route-specific tags we own (title/description/keywords/
  // canonical/og*/twitter*/robots/sitemap link/JSON-LD) — keep everything
  // else (favicon, viewport, charset, theme-color defaults, vite asset tags).
  const ownedTagPatterns = [
    /<title>[\s\S]*?<\/title>\s*/g,
    /<meta\s+name="description"[^>]*>\s*/g,
    /<meta\s+name="keywords"[^>]*>\s*/g,
    /<meta\s+name="theme-color"[^>]*>\s*/g,
    /<meta\s+name="application-name"[^>]*>\s*/g,
    /<meta\s+name="author"[^>]*>\s*/g,
    /<link\s+rel="canonical"[^>]*>\s*/g,
    /<link\s+rel="author"[^>]*>\s*/g,
    /<meta\s+name="robots"[^>]*>\s*/g,
    /<meta\s+name="googlebot"[^>]*>\s*/g,
    /<meta\s+name="bingbot"[^>]*>\s*/g,
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

// 1. Per-route HTML (homepage overwrites dist/index.html in place; every
// `/docs/<slug>` and `/manual/<slug>` gets its own pre-rendered shell).
let writtenCount = 0;
for (const route of allRoutes) {
  const html = spliceHead(shell, route);
  if (route.path === "/") {
    fs.writeFileSync(path.join(distDir, "index.html"), html);
    writtenCount++;
    continue;
  }
  const dir = path.join(distDir, route.path.replace(/^\//, ""));
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "index.html"), html);
  writtenCount++;
}
console.log(`wrote ${writtenCount} per-route HTML files (incl. ${docRoutes.length} docs, ${manRoutes.length} manpages)`);

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

// Priority and changefreq are hints, not commands — we set the homepage
// highest, top-level indexes next, and per-page references slightly lower
// so the homepage wins ties when crawlers ration their budget.
const sitemapEntries = [
  {
    loc: siteUrl,
    lastmod: lastmodFor(path.join(repoRoot, "README.md")),
    priority: "1.0",
    changefreq: "weekly",
  },
  {
    loc: `${siteUrl}docs`,
    lastmod: lastmodFor(docsDir),
    priority: "0.9",
    changefreq: "weekly",
  },
  {
    loc: `${siteUrl}manual`,
    lastmod: lastmodFor(manDir),
    priority: "0.9",
    changefreq: "weekly",
  },
  ...docsSlugs.map((s) => ({
    loc: `${siteUrl}docs/${s}`,
    lastmod: lastmodFor(path.join(docsDir, `${s}.md`)),
    priority: "0.7",
    changefreq: "monthly",
  })),
  ...manSlugs.map((s) => ({
    loc: `${siteUrl}manual/${s}`,
    lastmod: lastmodFor(path.join(manDir, `${s}.md`)),
    priority: "0.7",
    changefreq: "monthly",
  })),
];

const sitemap =
  `<?xml version="1.0" encoding="UTF-8"?>\n` +
  `<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n` +
  sitemapEntries
    .map(
      (u) =>
        `  <url>\n    <loc>${escape(u.loc)}</loc>\n    <lastmod>${u.lastmod}</lastmod>\n    <changefreq>${u.changefreq}</changefreq>\n    <priority>${u.priority}</priority>\n  </url>`,
    )
    .join("\n") +
  `\n</urlset>\n`;

fs.writeFileSync(path.join(distDir, "sitemap.xml"), sitemap);
console.log(`wrote dist/sitemap.xml (${sitemapEntries.length} URLs)`);

// 4. Smoke check (per OSS_SPEC §11.3 CI verification): bail out if the
// homepage is missing any required SEO element. Also verify a sampled
// sub-page received its own canonical so we never silently regress to
// the homepage shell for indexable URLs.
const home = fs.readFileSync(path.join(distDir, "index.html"), "utf8");
const required = [
  ["<title>", "homepage <title>"],
  [`<link rel="canonical"`, "canonical link"],
  [`application/ld+json`, "JSON-LD block"],
  [`og:image`, "og:image meta"],
  [`twitter:card`, "twitter card"],
  [`name="description"`, "meta description"],
  [`name="keywords"`, "meta keywords"],
];
const missing = required.filter(([needle]) => !home.includes(needle));
if (missing.length) {
  throw new Error(
    `Post-build SEO check failed — missing: ${missing.map(([, label]) => label).join(", ")}`,
  );
}
if (docRoutes.length > 0) {
  const sampleSlug = docRoutes[0].path.replace(/^\//, "");
  const sample = fs.readFileSync(path.join(distDir, sampleSlug, "index.html"), "utf8");
  const expectedCanonical = `${siteUrl}${sampleSlug}`;
  if (!sample.includes(`href="${expectedCanonical}"`)) {
    throw new Error(
      `Post-build SEO check failed — ${sampleSlug}/index.html is missing canonical ${expectedCanonical}`,
    );
  }
}

// Nudge the maintainer if the OG image asset is missing — crawlers will
// still index the page, but Slack/LinkedIn/Twitter previews will look
// broken. We warn rather than fail because the asset is generated
// out-of-band (per OSS_SPEC §11.3 it should ship under
// website/public/og-default.png).
const ogPath = path.join(distDir, "og-default.png");
if (!fs.existsSync(ogPath)) {
  console.warn(
    `WARNING: ${ogPath} is missing. Add a 1200x630 PNG at website/public/og-default.png ` +
      `so social previews on Slack/LinkedIn/Twitter render correctly.`,
  );
}

console.log("SEO smoke check OK");

// Make tooling that imports siteConfig with the file URL helper happy
// even though we don't use the import itself.
void pathToFileURL;
