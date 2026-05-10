import { Link } from "react-router-dom";
import { version, generatedAt } from "../data/sourceData";
import { repoUrl, cratesCliUrl, cratesLibUrl } from "../seo/siteConfig";

const generatedDate = new Date(generatedAt).toISOString().slice(0, 10);

export default function Footer() {
  return (
    <footer className="border-t border-border py-12">
      <div className="mx-auto max-w-6xl px-6">
        <div className="flex flex-col items-center justify-between gap-6 md:flex-row">
          <div>
            <span className="text-lg font-bold text-text-primary">
              <span className="text-accent" aria-hidden>🔌</span> zad
            </span>
            <p className="mt-1 text-sm text-text-dim">
              Connect AI agents to external services without an MCP server.
            </p>
          </div>

          <div className="flex flex-wrap justify-center gap-x-6 gap-y-2 text-sm text-text-secondary">
            <a href={repoUrl} target="_blank" rel="noopener noreferrer" className="transition-colors hover:text-text-primary">GitHub</a>
            <Link to="/docs/getting-started" className="transition-colors hover:text-text-primary">Documentation</Link>
            <Link to="/manual" className="transition-colors hover:text-text-primary">Manual</Link>
            <a href={cratesCliUrl} target="_blank" rel="noopener noreferrer" className="transition-colors hover:text-text-primary">crates.io · zad-cli</a>
            <a href={cratesLibUrl} target="_blank" rel="noopener noreferrer" className="transition-colors hover:text-text-primary">crates.io · zad</a>
            <a href={`${repoUrl}/blob/main/CHANGELOG.md`} target="_blank" rel="noopener noreferrer" className="transition-colors hover:text-text-primary">Changelog</a>
            <a href={`${repoUrl}/blob/main/LICENSE`} target="_blank" rel="noopener noreferrer" className="transition-colors hover:text-text-primary">MIT licence</a>
          </div>
        </div>

        <p className="mt-8 text-center text-xs text-text-dim">
          v{version} · generated {generatedDate} · content extracted from the repository at build time
        </p>
      </div>
    </footer>
  );
}
