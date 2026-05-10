// Runtime <head> updates for SPA navigation.
//
// Crawlers see the post-build splicer's pre-rendered HTML for every
// enumerated route, so SEO copy is correct on first paint. This module
// keeps the *human-visible* tabs (browser title) and the *shareable URL
// bar state* (canonical link, og:url) in sync as the user clicks
// around. It is deliberately small — no Helmet, no provider, no
// dependency on react-router's data routers — so any component can
// just `useDocumentHead({ title, description, canonicalPath })` from
// inside `useEffect`.

import { useEffect } from "react";
import { siteUrl } from "./siteConfig";

function setMeta(selector: string, attr: string, value: string) {
  let el = document.head.querySelector(selector) as HTMLMetaElement | null;
  if (!el) {
    el = document.createElement("meta");
    const [, name] = selector.match(/\[(?:name|property)="([^"]+)"\]/) ?? [];
    if (selector.includes('property="')) el.setAttribute("property", name ?? "");
    else el.setAttribute("name", name ?? "");
    document.head.appendChild(el);
  }
  el.setAttribute(attr, value);
}

function setLink(rel: string, href: string) {
  let el = document.head.querySelector(`link[rel="${rel}"]`) as HTMLLinkElement | null;
  if (!el) {
    el = document.createElement("link");
    el.setAttribute("rel", rel);
    document.head.appendChild(el);
  }
  el.setAttribute("href", href);
}

interface DocumentHead {
  title: string;
  description?: string;
  canonicalPath?: string;
}

export function useDocumentHead({ title, description, canonicalPath }: DocumentHead) {
  useEffect(() => {
    document.title = title;

    if (description) {
      setMeta('meta[name="description"]', "content", description);
      setMeta('meta[property="og:description"]', "content", description);
      setMeta('meta[name="twitter:description"]', "content", description);
    }

    if (canonicalPath !== undefined) {
      const url =
        canonicalPath === "/" || canonicalPath === ""
          ? siteUrl
          : `${siteUrl}${canonicalPath.replace(/^\//, "")}`;
      setLink("canonical", url);
      setMeta('meta[property="og:url"]', "content", url);
      setMeta('meta[property="og:title"]', "content", title);
      setMeta('meta[name="twitter:title"]', "content", title);
    }
  }, [title, description, canonicalPath]);
}
