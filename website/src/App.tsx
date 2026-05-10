import { useEffect } from "react";
import { Route, Routes, useLocation } from "react-router-dom";
import Navbar from "./components/Navbar";
import Hero from "./components/Hero";
import Features from "./components/Features";
import Services from "./components/Services";
import Permissions from "./components/Permissions";
import Library from "./components/Library";
import GetStarted from "./components/GetStarted";
import Footer from "./components/Footer";
import Documentation from "./components/Documentation";
import Manual from "./components/Manual";
import { siteDescription, siteName, siteTagline } from "./seo/siteConfig";
import { useDocumentHead } from "./seo/runtime";

function LandingPage() {
  useDocumentHead({
    title: `${siteName} — ${siteTagline}`,
    description: siteDescription,
    canonicalPath: "/",
  });
  return (
    <>
      <Hero />
      <Features />
      <Services />
      <Permissions />
      <Library />
      <GetStarted />
    </>
  );
}

// Simple smooth-scroll-to-hash on navigation (so /#features from the
// docs page lands at the right section after the route mount).
function ScrollToHash() {
  const location = useLocation();
  useEffect(() => {
    if (location.hash) {
      const el = document.querySelector(location.hash);
      if (el) {
        el.scrollIntoView({ behavior: "smooth", block: "start" });
        return;
      }
    }
    if (location.pathname === "/") {
      window.scrollTo({ top: 0, behavior: "auto" });
    }
  }, [location]);
  return null;
}

export default function App() {
  return (
    <div className="min-h-screen overflow-x-hidden bg-surface">
      <ScrollToHash />
      <Navbar />
      <Routes>
        <Route path="/" element={<LandingPage />} />
        <Route path="/docs" element={<Documentation />} />
        <Route path="/docs/:slug" element={<Documentation />} />
        <Route path="/manual" element={<Manual />} />
        <Route path="/manual/:slug" element={<Manual />} />
      </Routes>
      <Footer />
    </div>
  );
}
