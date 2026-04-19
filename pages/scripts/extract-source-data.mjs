// Extract project metadata from source so the pages site never goes stale
// (OSS_SPEC §11.2). Inputs are the repo's sources of truth:
//
//   - Cargo.toml              — package version + description
//   - src/cli/*.rs            — clap Subcommand enums, scraped for
//                               command paths + doc comments (no binary
//                               dependency, so this runs fine on a
//                               stock Node runner without Rust installed)
//   - README.md               — quick-start fenced block
//   - CHANGELOG.md            — latest release entry
//   - docs/                   — list of topic docs
//
// Output: pages/src/generated/sourceData.json (gitignored).

import fs from "node:fs";
import path from "node:path";
import { parse as parseToml } from "smol-toml";

const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..", "..");

function readTextFromRepo(...parts) {
  return fs.readFileSync(path.join(repoRoot, ...parts), "utf8");
}

function extractCargoMetadata() {
  const manifest = parseToml(readTextFromRepo("Cargo.toml"));
  return {
    version: manifest.package.version,
    description: manifest.package.description,
    repository: manifest.package.repository,
    license: manifest.package.license,
    rustVersion: manifest.package["rust-version"],
  };
}

function extractQuickStart() {
  const readme = readTextFromRepo("README.md");
  const heading = readme.indexOf("## Quick start");
  if (heading === -1) {
    throw new Error("README.md has no `## Quick start` section — extractor cannot continue.");
  }
  const fenceStart = readme.indexOf("```", heading);
  const langEnd = readme.indexOf("\n", fenceStart + 1);
  const fenceEnd = readme.indexOf("```", langEnd + 1);
  if (fenceStart === -1 || fenceEnd === -1) {
    throw new Error("README.md Quick start has no fenced code block.");
  }
  return readme.slice(langEnd + 1, fenceEnd).trim();
}

function extractLatestChangelogEntry() {
  const changelog = readTextFromRepo("CHANGELOG.md");
  const lines = changelog.split("\n");
  let start = -1;
  let end = -1;
  for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^## \[(v[^\]]+)\]/);
    if (m) {
      if (start === -1) {
        start = i;
      } else {
        end = i;
        break;
      }
    }
  }
  if (start === -1) {
    return null;
  }
  const slice = lines.slice(start, end === -1 ? lines.length : end).join("\n").trim();
  const headingMatch = slice.match(/^## \[(v[^\]]+)\]\s*(?:–\s*(\S+))?/);
  return {
    version: headingMatch ? headingMatch[1] : null,
    date: headingMatch && headingMatch[2] ? headingMatch[2] : null,
    body: slice,
  };
}

function extractDocsList() {
  const dir = path.join(repoRoot, "docs");
  return fs
    .readdirSync(dir)
    .filter((f) => f.endsWith(".md"))
    .map((f) => f.replace(/\.md$/, ""))
    .sort();
}

// Scrape the clap CLI tree out of src/cli/*.rs.
//
// Strategy: walk the enum graph starting at `Command` in src/cli/mod.rs.
// For each variant, read the `///` docs as the description, kebab-case
// the ident (honoring `#[command(name = "X")]` overrides), then look at
// the variant payload's struct for a `#[command(subcommand)]` field and
// recurse into the referenced enum.
//
// This reproduces the `{path, description}` subset of the binary's
// `zad commands --json` output — everything build.mjs actually consumes.
// The CI `pages-staleness` job still runs the binary extractor to catch
// drift (OSS_SPEC §11.2).
function extractCommands() {
  const cliDir = path.join(repoRoot, "src", "cli");
  const files = fs.readdirSync(cliDir).filter((f) => f.endsWith(".rs"));
  const sources = new Map();
  for (const f of files) {
    sources.set(f.replace(/\.rs$/, ""), fs.readFileSync(path.join(cliDir, f), "utf8"));
  }

  for (const [mod, src] of sources) {
    if (/#\[command\([^)]*\brename_all\b/.test(src)) {
      throw new Error(
        `src/cli/${mod}.rs uses #[command(rename_all = ...)]; the pages extractor ` +
          "only knows clap's default kebab-case. Extend extract-source-data.mjs to handle it.",
      );
    }
  }

  const enumIndex = new Map();
  const structIndex = new Map();
  // Allow arbitrary intervening attributes between `#[derive(...)]` and
  // the enum/struct header (e.g. `#[allow(clippy::large_enum_variant)]`).
  const enumRe = /#\[derive\(([^)]*)\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\s+)?enum\s+(\w+)\s*\{([\s\S]*?)\n\}/g;
  const structRe = /#\[derive\(([^)]*)\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\s+)?struct\s+(\w+)\s*\{([\s\S]*?)\n\}/g;
  for (const [mod, src] of sources) {
    for (const m of src.matchAll(enumRe)) {
      if (!/\bSubcommand\b/.test(m[1])) continue;
      enumIndex.set(`${mod}::${m[2]}`, { mod, body: m[3] });
    }
    for (const m of src.matchAll(structRe)) {
      structIndex.set(`${mod}::${m[2]}`, { mod, body: m[3] });
    }
  }

  function resolveIn(mod, typeRef, index) {
    let inner = typeRef.trim();
    const opt = inner.match(/^Option<\s*(.+?)\s*>$/);
    if (opt) inner = opt[1];
    if (inner.includes("::")) {
      const [m, n] = inner.split("::");
      return index.has(`${m}::${n}`) ? `${m}::${n}` : null;
    }
    return index.has(`${mod}::${inner}`) ? `${mod}::${inner}` : null;
  }

  function parseVariants(body) {
    const lines = body.split("\n");
    const variants = [];
    let pendingDocs = [];
    let pendingAttrs = [];
    for (const raw of lines) {
      const line = raw.trim();
      if (line === "") continue;
      const doc = line.match(/^\/\/\/\s?(.*)$/);
      if (doc) {
        pendingDocs.push(doc[1]);
        continue;
      }
      const attr = line.match(/^#\[(.+)\]$/);
      if (attr) {
        pendingAttrs.push(attr[1]);
        continue;
      }
      const v = line.match(/^([A-Z]\w*)\s*(?:\(([^)]*)\))?\s*[,{]/);
      if (v) {
        variants.push({
          ident: v[1],
          payload: v[2] ? v[2].trim() : null,
          docs: pendingDocs.join(" ").trim(),
          attrs: pendingAttrs,
        });
        pendingDocs = [];
        pendingAttrs = [];
      }
    }
    return variants;
  }

  function kebab(ident) {
    return ident
      .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
      .replace(/([A-Z]+)([A-Z][a-z])/g, "$1-$2")
      .toLowerCase();
  }

  const commands = [];
  function walk(enumKey, parent) {
    const entry = enumIndex.get(enumKey);
    if (!entry) throw new Error(`clap enum not found: ${enumKey}`);
    for (const v of parseVariants(entry.body)) {
      if (v.attrs.some((a) => /^command\([^)]*\bhide\s*=\s*true\b/.test(a))) continue;
      let cliName = kebab(v.ident);
      for (const a of v.attrs) {
        const m = a.match(/^command\([^)]*\bname\s*=\s*"([^"]+)"/);
        if (m) {
          cliName = m[1];
          break;
        }
      }
      const cmdPath = [...parent, cliName];
      // clap strips a single trailing period from doc-derived `about` text.
      const description = v.docs ? v.docs.replace(/\.$/, "") : null;
      commands.push({ path: cmdPath, description });
      if (!v.payload) continue;
      const structKey = resolveIn(entry.mod, v.payload, structIndex);
      if (!structKey) continue;
      const struct = structIndex.get(structKey);
      const field = struct.body.match(
        /#\[command\(subcommand\)\]\s*(?:#\[[^\]]*\]\s*)*pub\s+\w+\s*:\s*([\w:<>\s]+?)\s*,/,
      );
      if (!field) continue;
      const nested = resolveIn(struct.mod, field[1], enumIndex);
      if (nested) walk(nested, cmdPath);
    }
  }
  walk("mod::Command", []);
  return { commands };
}

const sourceData = {
  name: "zad",
  generatedAt: new Date().toISOString(),
  cargo: extractCargoMetadata(),
  quickStart: extractQuickStart(),
  changelog: extractLatestChangelogEntry(),
  docs: extractDocsList(),
  commands: extractCommands(),
};

const dest = path.join(repoRoot, "pages", "src", "generated");
fs.mkdirSync(dest, { recursive: true });
const outPath = path.join(dest, "sourceData.json");
fs.writeFileSync(outPath, JSON.stringify(sourceData, null, 2));
console.log("wrote", path.relative(repoRoot, outPath));
