// Extract project metadata from source so the website never goes stale
// (OSS_SPEC §11.2). Inputs are the repo's sources of truth:
//
//   - Cargo.toml                                — workspace version, license, MSRV
//   - crates/zad-cli/Cargo.toml                 — binary user-facing description
//   - crates/zad/Cargo.toml                     — library description
//   - crates/zad/src/service/registry.rs        — canonical service list
//   - ./target/debug/zad commands --json        — full clap command tree
//   - README.md                                  — quick-start + library snippet
//   - CHANGELOG.md                               — latest release entry
//   - docs/                                      — list of topic docs
//   - examples/                                  — list of runnable examples
//
// Output: website/src/generated/sourceData.json (gitignored).
//
// Failure mode (per OSS_SPEC §11.2): if any expected marker is missing
// the script throws a descriptive error so the website build fails
// loudly instead of shipping stale data.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { parse as parseToml } from "smol-toml";

const repoRoot = path.resolve(
  path.dirname(new URL(import.meta.url).pathname),
  "..",
  "..",
);

function readRepoFile(...parts) {
  return fs.readFileSync(path.join(repoRoot, ...parts), "utf8");
}

function extractCargoMetadata() {
  const root = parseToml(readRepoFile("Cargo.toml"));
  const wp = root.workspace && root.workspace.package;
  if (!wp) {
    throw new Error(
      "Cargo.toml has no [workspace.package] table — extractor cannot read shared metadata.",
    );
  }
  const cli = parseToml(readRepoFile("crates", "zad-cli", "Cargo.toml"));
  if (!cli.package?.description) {
    throw new Error(
      "crates/zad-cli/Cargo.toml has no package.description — extractor cannot read CLI marketing copy.",
    );
  }
  const lib = parseToml(readRepoFile("crates", "zad", "Cargo.toml"));
  if (!lib.package?.description) {
    throw new Error(
      "crates/zad/Cargo.toml has no package.description — extractor cannot read library description.",
    );
  }
  return {
    version: wp.version,
    license: wp.license,
    repository: wp.repository,
    rustVersion: wp["rust-version"],
    edition: wp.edition,
    cliDescription: cli.package.description,
    libDescription: lib.package.description,
  };
}

function extractServices() {
  const src = readRepoFile("crates", "zad", "src", "service", "registry.rs");
  const m = src.match(/pub const SERVICES:\s*&\[&str\]\s*=\s*&\[([^\]]+)\];/);
  if (!m) {
    throw new Error(
      "crates/zad/src/service/registry.rs no longer matches `pub const SERVICES: &[&str] = &[...]` — extractor needs updating.",
    );
  }
  const names = m[1]
    .split(",")
    .map((s) => s.trim().replace(/^"|"$/g, ""))
    .filter(Boolean);
  if (names.length === 0) {
    throw new Error(
      "registry.rs SERVICES list parsed as empty — refusing to ship a website with no services.",
    );
  }
  return names;
}

function extractQuickStart() {
  const readme = readRepoFile("README.md");
  const heading = readme.indexOf("## Quick start");
  if (heading === -1) {
    throw new Error("README.md has no `## Quick start` section.");
  }
  const fenceStart = readme.indexOf("```", heading);
  const langEnd = readme.indexOf("\n", fenceStart + 1);
  const fenceEnd = readme.indexOf("```", langEnd + 1);
  if (fenceStart === -1 || fenceEnd === -1) {
    throw new Error("README.md `## Quick start` is missing a fenced code block.");
  }
  return readme.slice(langEnd + 1, fenceEnd).trim();
}

function extractLibrarySnippet() {
  const readme = readRepoFile("README.md");
  const heading = readme.indexOf("### Use as a library");
  if (heading === -1) {
    throw new Error("README.md has no `### Use as a library` section.");
  }
  // Grab the first ```rust fenced block after the heading.
  const fenceStart = readme.indexOf("```rust", heading);
  if (fenceStart === -1) {
    throw new Error("README.md `### Use as a library` has no ```rust block.");
  }
  const langEnd = readme.indexOf("\n", fenceStart + 1);
  const fenceEnd = readme.indexOf("```", langEnd + 1);
  return readme.slice(langEnd + 1, fenceEnd).trim();
}

function extractLatestChangelogEntry() {
  const changelog = readRepoFile("CHANGELOG.md");
  const lines = changelog.split("\n");
  let start = -1;
  let end = -1;
  for (let i = 0; i < lines.length; i++) {
    if (/^## \[/.test(lines[i])) {
      if (start === -1) start = i;
      else {
        end = i;
        break;
      }
    }
  }
  if (start === -1) return null;
  const slice = lines
    .slice(start, end === -1 ? lines.length : end)
    .join("\n")
    .trim();
  const headingMatch = slice.match(/^## \[([^\]]+)\]\s*(?:[–-]\s*(\S+))?/);
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

function extractManpageList() {
  const dir = path.join(repoRoot, "man");
  return fs
    .readdirSync(dir)
    .filter((f) => f.endsWith(".md"))
    .map((f) => f.replace(/\.md$/, ""))
    .sort();
}

function extractExamplesList() {
  const dir = path.join(repoRoot, "examples");
  return fs
    .readdirSync(dir, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort();
}

function extractCommandTree() {
  const binary = path.join(
    repoRoot,
    "target",
    "debug",
    process.platform === "win32" ? "zad.exe" : "zad",
  );
  if (!fs.existsSync(binary)) {
    throw new Error(
      `zad binary not found at ${binary}. Run \`cargo build --bin zad\` before the extractor ` +
        "(or invoke `make website`, which does it for you).",
    );
  }
  const stdout = execFileSync(binary, ["commands", "--json"], { encoding: "utf8" });
  return JSON.parse(stdout);
}

const cargo = extractCargoMetadata();
const services = extractServices();
const commands = extractCommandTree();

// Drift check: every name in registry.rs must show up as a top-level
// command in the binary, otherwise our service grid will be wrong.
const topLevelCommands = new Set(
  (commands.commands || []).map((c) => c.path?.[0] || c.name),
);
for (const svc of services) {
  if (!topLevelCommands.has(svc)) {
    throw new Error(
      `Service "${svc}" is listed in registry.rs but missing from \`zad commands\`. ` +
        "The website cannot link to its CLI surface — fix registry/CLI wiring or update the extractor.",
    );
  }
}

const sourceData = {
  name: "zad",
  generatedAt: new Date().toISOString(),
  cargo,
  services,
  commands,
  quickStart: extractQuickStart(),
  librarySnippet: extractLibrarySnippet(),
  changelog: extractLatestChangelogEntry(),
  docs: extractDocsList(),
  manpages: extractManpageList(),
  examples: extractExamplesList(),
};

const dest = path.join(repoRoot, "website", "src", "generated");
fs.mkdirSync(dest, { recursive: true });
const outPath = path.join(dest, "sourceData.json");
fs.writeFileSync(outPath, JSON.stringify(sourceData, null, 2));
console.log("wrote", path.relative(repoRoot, outPath));
