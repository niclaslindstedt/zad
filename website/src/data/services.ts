// Display metadata for each shipped service. Names must match the
// canonical list parsed from `crates/zad/src/service/registry.rs` by
// the extractor — `assertServicesMatch` below verifies that at module
// load and throws if anyone forgets to update both sides.

import { services as registryServices } from "./sourceData";

export interface ServiceCard {
  name: string;
  displayName: string;
  tagline: string;
  auth: string;
  verbs: string[];
  colorVar: string;
  manpage: string;
}

export const serviceCards: ServiceCard[] = [
  {
    name: "discord",
    displayName: "Discord",
    tagline:
      "Send and read messages, list channels, DM yourself, walk guilds — name → snowflake cache means agents talk in human names.",
    auth: "Bot token (OS keychain)",
    verbs: ["send", "read", "channels", "join", "leave", "discover"],
    colorVar: "text-svc-discord",
    manpage: "discord",
  },
  {
    name: "slack",
    displayName: "Slack",
    tagline:
      "Web API + Socket Mode listener. Channel/DM addressing by name or ID, Block Kit messages, dry-run preview.",
    auth: "Bot + (optional) App-Level token",
    verbs: ["send", "read", "channels", "discover", "directory"],
    colorVar: "text-svc-slack",
    manpage: "slack",
  },
  {
    name: "telegram",
    displayName: "Telegram",
    tagline:
      "Bot API send/read with signed chat IDs and @username addressing. Discover seeds the directory cache.",
    auth: "Bot token (OS keychain)",
    verbs: ["send", "read", "chats", "discover"],
    colorVar: "text-svc-telegram",
    manpage: "telegram",
  },
  {
    name: "gcal",
    displayName: "Google Calendar",
    tagline:
      "List, create, and update events with attendee, time-window, notice-period, and shared-calendar guards.",
    auth: "OAuth 2.0 desktop loopback (PKCE)",
    verbs: ["calendars", "events", "permissions", "self"],
    colorVar: "text-svc-gcal",
    manpage: "gcal",
  },
  {
    name: "spotify",
    displayName: "Spotify",
    tagline:
      "Search, manage playlists, and curate your library. Public-client PKCE flow — no client secret needed.",
    auth: "OAuth 2.0 PKCE public client",
    verbs: ["search", "playlists", "library"],
    colorVar: "text-svc-spotify",
    manpage: "spotify",
  },
  {
    name: "ymusic",
    displayName: "YouTube Music",
    tagline:
      "Search, playlists, and likes via the YouTube Data API v3. Same Google OAuth shape as Calendar.",
    auth: "OAuth 2.0 desktop loopback",
    verbs: ["search", "playlists", "library"],
    colorVar: "text-svc-ymusic",
    manpage: "ymusic",
  },
  {
    name: "1pass",
    displayName: "1Password",
    tagline:
      "Read-only by default with hidden-target semantics: out-of-scope items present as if they don't exist. Wraps the official op CLI; destructive verbs are not exposed.",
    auth: "Service-account token (OS keychain)",
    verbs: ["vaults", "items", "get", "read", "inject", "create", "whoami"],
    colorVar: "text-svc-1pass",
    manpage: "1pass",
  },
];

// Drift guard: if registry.rs gains or loses a service the website
// build fails until this list is updated to match.
function assertServicesMatch(): void {
  const have = new Set(registryServices);
  const declared = new Set(serviceCards.map((s) => s.name));
  const missing = registryServices.filter((s) => !declared.has(s));
  const extra = serviceCards.map((s) => s.name).filter((s) => !have.has(s));
  if (missing.length || extra.length) {
    throw new Error(
      `services.ts is out of sync with crates/zad/src/service/registry.rs. ` +
        `Missing display copy for: ${missing.join(", ") || "(none)"}. ` +
        `Stale entries: ${extra.join(", ") || "(none)"}. ` +
        `Update website/src/data/services.ts to match.`,
    );
  }
}

assertServicesMatch();
