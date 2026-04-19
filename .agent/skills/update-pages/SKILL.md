---
name: update-pages
description: "Use when the pages site may be stale. Discovers commits since the last pages update and refreshes source-derived content under pages/ so generated pages match the current README, docs, and spec."
---

# Updating the Pages Site

The `pages/` directory contains the GitHub Pages site for `zad`. Per §11.2 of `OSS_SPEC.md`, its source-derived content (hero copy, feature lists, version strings) must not be authored twice — it is extracted from `README.md`, `docs/`, `OSS_SPEC.md`, and the clap CLI source, then rendered by the pages build.

## Tracking mechanism

`.agent/skills/update-pages/.last-updated` contains the git commit hash from the last successful run. Empty means "never run" — fall back to the initial commit.

## Discovery process

1. Read the baseline:

   ```sh
   BASELINE=$(cat .agent/skills/update-pages/.last-updated)
   ```

2. Diff sources of truth against the baseline:

   ```sh
   git log --oneline "$BASELINE"..HEAD -- README.md docs/ OSS_SPEC.md src/cli/
   git diff --name-only "$BASELINE"..HEAD -- README.md docs/ OSS_SPEC.md src/cli/
   ```

3. If anything changed, rebuild the pages site and inspect the diff under `pages/`.

## Mapping table

| Changed file | Effect on pages site |
|---|---|
| `README.md` hero / quick start | Home page feature summary |
| `README.md` Usage / install | Install & usage pages |
| `docs/getting-started.md` | "Getting started" page |
| `OSS_SPEC.md` front-matter `version:` | Version badge on the home page |
| `src/cli/*.rs` Subcommand enums | Command list on the home page |

## Update checklist

- [ ] Read baseline and diff sources of truth
- [ ] Refresh generated content under `pages/`
- [ ] Build the pages site locally and smoke-test the home page
- [ ] Confirm the §11.2 staleness CI check would pass
- [ ] Write the new baseline:

      git rev-parse HEAD > .agent/skills/update-pages/.last-updated

## Verification

1. Open the rendered site locally and verify hero copy, version, and key tables.
2. Run the pages staleness CI check from §11.2 against HEAD.
3. Confirm `.last-updated` was rewritten.

## Skill self-improvement

1. **Expand the mapping table** if a new source file started feeding the pages site.
2. **Record extraction quirks** (e.g. "anchor X is parsed from heading Y").
3. **Commit the skill edit** alongside the pages update.
