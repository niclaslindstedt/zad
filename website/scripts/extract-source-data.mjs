// Extract project metadata from source so the website never goes stale
// (OSS_SPEC §11.2). Inputs are the repo's sources of truth:
//
//   - Cargo.toml              — package version + description
//   - ./target/debug/zad      — built binary, queried for commands
//   - README.md               — quick-start fenced block
//   - CHANGELOG.md            — latest release entry
//   - docs/                   — list of topic docs
//
// Output: website/src/generated/sourceData.json (gitignored).

import { execFileSync } from "node:child_process";
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
  // First version heading that is not "[Unreleased]".
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

function extractCommands() {
  const binary = path.join(repoRoot, "target", "debug", process.platform === "win32" ? "zad.exe" : "zad");
  if (!fs.existsSync(binary)) {
    throw new Error(
      `zad binary not found at ${binary}. Run \`cargo build --bin zad\` before the extractor ` +
        "(or invoke `make website`, which does it for you).",
    );
  }
  const stdout = execFileSync(binary, ["commands", "--json"], { encoding: "utf8" });
  return JSON.parse(stdout);
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

const dest = path.join(repoRoot, "website", "src", "generated");
fs.mkdirSync(dest, { recursive: true });
const outPath = path.join(dest, "sourceData.json");
fs.writeFileSync(outPath, JSON.stringify(sourceData, null, 2));
console.log("wrote", path.relative(repoRoot, outPath));
