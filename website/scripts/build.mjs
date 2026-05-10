// Render the static website from the extracted source data.
//
// The extractor (extract-source-data.mjs) writes
// website/src/generated/sourceData.json. This script renders a single
// HTML page at website/dist/index.html by filling a string template
// with the extracted values. Vite is still usable as a dev server, but
// the production deployable artifact is what this script writes.

import fs from "node:fs";
import path from "node:path";

const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..", "..");
const generated = path.join(repoRoot, "website", "src", "generated", "sourceData.json");
if (!fs.existsSync(generated)) {
  throw new Error(
    `sourceData.json is missing at ${generated}. Run \`npm run extract\` first ` +
      "(or just `npm run build`, which chains extract → build).",
  );
}

const data = JSON.parse(fs.readFileSync(generated, "utf8"));
const dist = path.join(repoRoot, "website", "dist");
fs.mkdirSync(dist, { recursive: true });

// Canonical absolute URL for the deployed site. GitHub Pages serves
// project sites under <user>.github.io/<repo>/, derived from the
// repository field in Cargo.toml.
const repoUrl = data.cargo.repository.replace(/\.git$/, "").replace(/\/$/, "");
const repoMatch = repoUrl.match(/github\.com\/([^/]+)\/([^/]+)$/);
const siteUrl = repoMatch
  ? `https://${repoMatch[1]}.github.io/${repoMatch[2]}/`
  : `${repoUrl}/`;
const ogImageUrl = `${siteUrl}og-default.png`;

function escape(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function renderCommandList(commands) {
  return commands
    .map((c) => {
      const path = escape(c.path.join(" "));
      const desc = escape(c.description || "");
      return `<li><code>zad ${path}</code> — ${desc}</li>`;
    })
    .join("\n        ");
}

const commands = data.commands.commands || [];
const latestRelease = data.changelog
  ? `<section>
      <h2>Latest release</h2>
      <h3>${escape(data.changelog.version || "")}${
        data.changelog.date ? ` — ${escape(data.changelog.date)}` : ""
      }</h3>
      <pre><code>${escape(data.changelog.body)}</code></pre>
    </section>`
  : "";

const pageTitle = `zad — ${data.cargo.description.split(".")[0]}`;
const jsonLd = {
  "@context": "https://schema.org",
  "@type": "SoftwareApplication",
  "@id": siteUrl,
  name: "zad",
  description: data.cargo.description,
  applicationCategory: "DeveloperApplication",
  operatingSystem: "Linux, macOS, Windows",
  softwareVersion: data.cargo.version,
  license: `https://spdx.org/licenses/${data.cargo.license}.html`,
  codeRepository: data.cargo.repository,
  url: siteUrl,
  image: ogImageUrl,
  offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
};

const html = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width,initial-scale=1" />
    <title>${escape(pageTitle)}</title>
    <meta name="description" content="${escape(data.cargo.description)}" />
    <link rel="canonical" href="${escape(siteUrl)}" />
    <meta name="robots" content="index,follow,max-image-preview:large" />
    <link rel="sitemap" type="application/xml" href="sitemap.xml" />
    <!-- Crawler policy: see robots.txt at site root. -->
    <meta property="og:site_name" content="zad" />
    <meta property="og:type" content="website" />
    <meta property="og:title" content="${escape(pageTitle)}" />
    <meta property="og:description" content="${escape(data.cargo.description)}" />
    <meta property="og:url" content="${escape(siteUrl)}" />
    <meta property="og:image" content="${escape(ogImageUrl)}" />
    <meta property="og:image:width" content="1200" />
    <meta property="og:image:height" content="630" />
    <meta property="og:image:alt" content="zad — ${escape(data.cargo.description.split(".")[0])}" />
    <meta name="twitter:card" content="summary_large_image" />
    <meta name="twitter:title" content="${escape(pageTitle)}" />
    <meta name="twitter:description" content="${escape(data.cargo.description)}" />
    <meta name="twitter:image" content="${escape(ogImageUrl)}" />
    <script type="application/ld+json">${JSON.stringify(jsonLd)}</script>
    <style>
      body { font-family: system-ui, -apple-system, "Segoe UI", sans-serif; max-width: 820px; margin: 2rem auto; padding: 0 1rem; line-height: 1.55; }
      header { border-bottom: 1px solid #e5e7eb; padding-bottom: 1rem; margin-bottom: 1.5rem; }
      h1 { margin: 0; }
      pre { background: #0b1020; color: #e5e7eb; padding: 1rem; border-radius: 6px; overflow-x: auto; }
      code { font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; }
      section { margin: 2rem 0; }
      .meta { color: #6b7280; font-size: 0.9rem; }
      ul { padding-left: 1.2rem; }
      li { margin: 0.25rem 0; }
    </style>
  </head>
  <body>
    <header>
      <h1>zad <span class="meta">v${escape(data.cargo.version)}</span></h1>
      <p>${escape(data.cargo.description)}</p>
      <p class="meta">Rust ${escape(data.cargo.rustVersion)}+ · ${escape(data.cargo.license)} · <a href="${escape(
        data.cargo.repository,
      )}">Source on GitHub</a></p>
    </header>

    <section>
      <h2>Install</h2>
      <pre><code>cargo install --path .</code></pre>
    </section>

    <section>
      <h2>Quick start</h2>
      <pre><code>${escape(data.quickStart)}</code></pre>
    </section>

    <section>
      <h2>Commands</h2>
      <ul>
        ${renderCommandList(commands)}
      </ul>
    </section>

    <section>
      <h2>Documentation</h2>
      <ul>
        ${data.docs.map((d) => `<li><code>zad docs ${escape(d)}</code></li>`).join("\n        ")}
      </ul>
    </section>

    ${latestRelease}

    <footer>
      <p class="meta">Generated ${escape(data.generatedAt)}.</p>
    </footer>
  </body>
</html>
`;

const outPath = path.join(dist, "index.html");
fs.writeFileSync(outPath, html);
console.log("wrote", path.relative(repoRoot, outPath));

const publicDir = path.join(repoRoot, "website", "public");
if (fs.existsSync(publicDir)) {
  for (const entry of fs.readdirSync(publicDir, { withFileTypes: true })) {
    if (!entry.isFile()) continue;
    fs.copyFileSync(path.join(publicDir, entry.name), path.join(dist, entry.name));
    console.log("copied", path.posix.join("website/public", entry.name), "→", path.posix.join("website/dist", entry.name));
  }
}
