import { useState, useEffect } from "react";
import { Link, useLocation } from "react-router-dom";
import { repoUrl } from "../seo/siteConfig";

export default function Navbar() {
  const location = useLocation();
  const [menuOpen, setMenuOpen] = useState(false);
  const isDocsPage = location.pathname.startsWith("/docs");
  const isManualPage = location.pathname.startsWith("/manual");
  const isSubPage = isDocsPage || isManualPage;

  useEffect(() => {
    setMenuOpen(false);
  }, [location.pathname]);

  const sectionHref = (hash: string) => (isSubPage ? `/${hash}` : hash);

  const navLinks = (
    <>
      <a href={sectionHref("#features")}     className="text-sm text-text-secondary hover:text-text-primary transition-colors">Features</a>
      <a href={sectionHref("#services")}     className="text-sm text-text-secondary hover:text-text-primary transition-colors">Services</a>
      <a href={sectionHref("#permissions")}  className="text-sm text-text-secondary hover:text-text-primary transition-colors">Permissions</a>
      <a href={sectionHref("#library")}      className="text-sm text-text-secondary hover:text-text-primary transition-colors">Library</a>
      <a href={sectionHref("#get-started")}  className="text-sm text-text-secondary hover:text-text-primary transition-colors">Install</a>
      <Link
        to="/docs/getting-started"
        className={`text-sm transition-colors ${isDocsPage ? "text-accent" : "text-text-secondary hover:text-text-primary"}`}
      >
        Docs
      </Link>
      <Link
        to="/manual"
        className={`text-sm transition-colors ${isManualPage ? "text-accent" : "text-text-secondary hover:text-text-primary"}`}
      >
        Manual
      </Link>
    </>
  );

  return (
    <nav className="fixed top-0 right-0 left-0 z-50 border-b border-border bg-surface/80 backdrop-blur-md">
      <div className="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
        <Link to="/" className="flex items-center gap-2 text-xl font-bold text-text-primary">
          <span className="text-accent" aria-hidden>🔌</span> zad
        </Link>

        <div className="hidden items-center gap-7 md:flex">{navLinks}</div>

        <div className="flex items-center gap-3">
          <button
            onClick={() => setMenuOpen(!menuOpen)}
            className="cursor-pointer p-1 text-text-secondary hover:text-text-primary transition-colors md:hidden"
            aria-label="Toggle menu"
          >
            <svg className="h-6 w-6" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d={menuOpen ? "M6 18L18 6M6 6l12 12" : "M4 6h16M4 12h16M4 18h16"} />
            </svg>
          </button>

          <a
            href={repoUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="rounded-lg border border-border px-4 py-2 text-sm text-text-secondary hover:border-accent hover:text-text-primary transition-all"
          >
            GitHub
          </a>
        </div>
      </div>

      {menuOpen && (
        <>
          <div className="fixed inset-0 top-[73px] z-40 bg-black/50 md:hidden" onClick={() => setMenuOpen(false)} />
          <div className="relative z-50 border-t border-border bg-surface/95 backdrop-blur-md px-6 py-4 md:hidden">
            <div className="flex flex-col gap-4" onClick={() => setMenuOpen(false)}>
              {navLinks}
            </div>
          </div>
        </>
      )}
    </nav>
  );
}
