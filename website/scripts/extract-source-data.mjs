// Extract project metadata from source so the website never goes stale.
// Outputs website/src/generated/sourceData.json.
import fs from "node:fs";
import path from "node:path";

const out = {
  name: "zad",
  description: "A Rust CLI that connects AI agents to external services (Discord, GitHub, Slack, etc.) via scoped adapter configurations instead of MCP servers.",
  generatedAt: new Date().toISOString(),
};

const dest = path.join("src", "generated");
fs.mkdirSync(dest, { recursive: true });
fs.writeFileSync(path.join(dest, "sourceData.json"), JSON.stringify(out, null, 2));
console.log("wrote", path.join(dest, "sourceData.json"));