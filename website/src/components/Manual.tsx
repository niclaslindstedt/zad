import { useEffect, useState } from "react";
import { Link, Navigate, useParams } from "react-router-dom";
import { manpages, getManpageBySlug } from "../data/manpages";
import MarkdownRenderer from "./MarkdownRenderer";

export default function Manual() {
  const { slug } = useParams<{ slug: string }>();
  const [sidebarOpen, setSidebarOpen] = useState(false);

  const currentSlug = slug || "main";
  const currentPage = getManpageBySlug(currentSlug);

  useEffect(() => {
    document.title = currentPage
      ? `${currentPage.title} — zad manual`
      : "Manual — zad";
    window.scrollTo(0, 0);
  }, [currentSlug, currentPage]);

  if (!currentPage) {
    return <Navigate to="/manual/main" replace />;
  }

  return (
    <div className="min-h-screen pt-[73px]">
      <div className="sticky top-[73px] z-40 border-b border-border bg-surface/95 backdrop-blur-sm px-4 py-3 lg:hidden">
        <button
          onClick={() => setSidebarOpen(!sidebarOpen)}
          className="flex items-center gap-2 text-sm text-text-secondary transition-colors hover:text-text-primary"
        >
          <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d={sidebarOpen ? "M6 18L18 6M6 6l12 12" : "M4 6h16M4 12h16M4 18h16"} />
          </svg>
          {currentPage.title}
        </button>
      </div>

      {sidebarOpen && (
        <div
          className="fixed inset-0 z-40 bg-black/50 lg:hidden"
          onClick={() => setSidebarOpen(false)}
        />
      )}

      <div className="mx-auto flex max-w-7xl">
        <aside
          className={`fixed top-[118px] bottom-0 z-40 w-full shrink-0 overflow-y-auto border-r border-border bg-surface px-4 py-6
            transition-transform duration-200 ease-in-out
            sm:w-72
            lg:sticky lg:top-[73px] lg:block lg:w-64 lg:translate-x-0
            ${sidebarOpen ? "translate-x-0" : "-translate-x-full"}
          `}
        >
          <div className="mb-2 px-3 text-xs font-semibold tracking-widest text-text-dim uppercase">
            Manual
          </div>
          <nav className="space-y-1">
            {manpages.map((m) => (
              <Link
                key={m.slug}
                to={`/manual/${m.slug}`}
                onClick={() => setSidebarOpen(false)}
                className={`block rounded-md px-3 py-2 font-mono text-[13px] transition-colors
                  ${m.slug === currentSlug
                    ? "bg-accent/10 font-medium text-accent"
                    : "text-text-secondary hover:bg-surface-hover hover:text-text-primary"
                  }`}
              >
                {m.title}
              </Link>
            ))}
          </nav>
        </aside>

        <main className="min-w-0 flex-1 px-6 py-8 lg:px-12 lg:py-10">
          <MarkdownRenderer content={currentPage.content} basePath="/manual" sourceDir="man" />
        </main>
      </div>
    </div>
  );
}
