import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Link } from "react-router-dom";
import type { Components } from "react-markdown";
import type { ReactNode } from "react";
import CodeBlock from "./CodeBlock";

interface MarkdownRendererProps {
  content: string;
  basePath?: string;
  /** Path of the source file relative to the repo root, used to rewrite
      relative `[..](other.md)` links to the right hosted route. */
  sourceDir?: "docs" | "man";
}

function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^\w\s-]/g, "")
    .replace(/\s+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

function extractText(children: ReactNode): string {
  if (typeof children === "string") return children;
  if (Array.isArray(children)) return children.map(extractText).join("");
  if (children && typeof children === "object" && "props" in children) {
    const props = (children as { props?: { children?: ReactNode } }).props;
    return extractText(props?.children);
  }
  return String(children ?? "");
}

function heading(Tag: "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
  return function HeadingComponent({ children }: { children?: ReactNode }) {
    const id = slugify(extractText(children));
    return <Tag id={id}>{children}</Tag>;
  };
}

function makeComponents(basePath: string, sourceDir: "docs" | "man"): Components {
  return {
    pre: CodeBlock,
    h1: heading("h1"),
    h2: heading("h2"),
    h3: heading("h3"),
    h4: heading("h4"),
    h5: heading("h5"),
    h6: heading("h6"),
    a({ href, children }) {
      if (!href) {
        return <a>{children}</a>;
      }

      // Rewrite "../man/foo.md" / "../docs/foo.md" to the hosted route.
      const manRel = href.match(/(?:^|\/)man\/([^/]+)\.md$/);
      if (manRel) {
        return (
          <Link to={`/manual/${manRel[1]}`} className="text-accent hover:text-accent-light transition-colors underline">
            {children}
          </Link>
        );
      }
      const docsRel = href.match(/(?:^|\/)docs\/([^/]+)\.md$/);
      if (docsRel) {
        return (
          <Link to={`/docs/${docsRel[1]}`} className="text-accent hover:text-accent-light transition-colors underline">
            {children}
          </Link>
        );
      }

      // Plain relative .md → assume same source-dir.
      if (href.endsWith(".md") && !href.includes(":")) {
        const slug = href.replace(/^\.\//, "").replace(/\.md$/, "");
        const target = sourceDir === "man" ? `/manual/${slug}` : `/docs/${slug}`;
        return (
          <Link to={`${basePath}/${slug}`.replace(`${basePath}/${slug}`, target)} className="text-accent hover:text-accent-light transition-colors underline">
            {children}
          </Link>
        );
      }

      // Repo-relative links (e.g. examples/, LICENSE) → GitHub.
      if (!href.startsWith("http") && !href.startsWith("#") && !href.startsWith("/")) {
        return (
          <a
            href={`https://github.com/niclaslindstedt/zad/blob/main/${href}`}
            className="text-accent hover:text-accent-light transition-colors underline"
            target="_blank"
            rel="noopener noreferrer"
          >
            {children}
          </a>
        );
      }

      const isExternal = href.startsWith("http://") || href.startsWith("https://");
      return (
        <a
          href={href}
          className="text-accent hover:text-accent-light transition-colors underline"
          {...(isExternal ? { target: "_blank", rel: "noopener noreferrer" } : {})}
        >
          {children}
        </a>
      );
    },
  };
}

export default function MarkdownRenderer({
  content,
  basePath = "/docs",
  sourceDir = "docs",
}: MarkdownRendererProps) {
  const components = makeComponents(basePath, sourceDir);
  return (
    <div className="markdown-content">
      <Markdown remarkPlugins={[remarkGfm]} components={components}>
        {content}
      </Markdown>
    </div>
  );
}
