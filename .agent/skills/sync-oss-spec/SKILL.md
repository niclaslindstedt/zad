---
name: sync-oss-spec
description: "Use when the repository may have drifted out of conformance with OSS_SPEC.md. Runs `oss-spec validate .` (or the bash fallback when the binary is unavailable), walks the violations, and fixes each one until the repo is back in sync."
---

# Syncing zad with OSS_SPEC.md

`OSS_SPEC.md` is the specification this repo claims to conform to (see the "OSS Spec conformance" section of `CLAUDE.md`), and `oss-spec validate .` is the machine-readable enforcement of that claim. Whenever a new file/directory/symlink mandate from the spec is not yet reflected on disk here, this skill is the playbook for closing that gap: it runs the validator, inspects each violation, and brings the repo back into conformance.

This skill is the final step of a drift sweep — `maintenance` (and the per-artifact `update-*` skills) handle drift in known surfaces; `sync-oss-spec` catches residual structural violations the per-artifact skills did not touch.

## Tracking mechanism

`.agent/skills/sync-oss-spec/.last-updated` contains the git commit hash of the last successful run. Empty means "never run" — use the repo's initial commit (`git rev-list --max-parents=0 HEAD`) as the baseline.

## Discovery process

1. Read the baseline:

   ```sh
   BASELINE=$(cat .agent/skills/sync-oss-spec/.last-updated)
   ```

2. Check whether `OSS_SPEC.md` changed since the baseline — that is the only input that can invalidate previously-passing conformance from inside this repo:

   ```sh
   git log --oneline "$BASELINE"..HEAD -- OSS_SPEC.md
   git diff --name-only "$BASELINE"..HEAD
   ```

3. Run the validator against the repo. This is the source of truth for what is currently out of spec:

   ```sh
   oss-spec validate --no-ai --path .   # structural violations only
   oss-spec validate --path .           # also runs AI quality review
   ```

   Each structural violation names the spec section (e.g. `§7.1`, `§10.3`, `§21.5`) and the file or directory at fault. AI findings name the file, the section, a severity, and a suggestion.

   **Nonbinary fallback.** If the `oss-spec` binary is not available in the current environment (sandboxed agent sessions, ephemeral CI without a Rust toolchain, freshly-cloned checkouts where `cargo install oss-spec` would be too slow), use the language-agnostic bash mirror published from the oss-spec repo. It implements the same deterministic §19 checks and prints the AI quality checklist as a manual prompt at the end:

   ```sh
   curl -fsSL https://raw.githubusercontent.com/niclaslindstedt/oss-spec/main/scripts/validate.sh | bash -s -- .
   ```

   The bash mirror downloads the latest `OSS_SPEC.md` to the repo root (overwriting the existing copy so `git diff -- OSS_SPEC.md` shows what changed since the project was last brought into conformance), runs every deterministic §19 check, and ends with an LLM-directed checklist to work through file by file. Treat the rewritten `OSS_SPEC.md` as part of the discovery output — review the diff before committing so you do not silently bump the in-repo spec version without intending to.

4. For each violation, read the relevant section of `OSS_SPEC.md` so the fix matches the spec's intent rather than just silencing the check.

## Mapping table

| Violation spec section | Where to fix it |
|---|---|
| §2 missing `LICENSE` | Create `LICENSE` with the SPDX-identified license text and the correct copyright holder |
| §3 missing `README.md` sections | Edit `README.md`; run `update-readme` afterwards if extensive rewording is needed |
| §4/§5/§6 missing `CONTRIBUTING.md` / `CODE_OF_CONDUCT.md` / `SECURITY.md` | Create the file with the minimum content mandated by the corresponding spec section |
| §7.1 tool-specific guidance file is not a symlink | Replace the regular file with `ln -s CLAUDE.md <path>` (or `ln -s ../CLAUDE.md .github/copilot-instructions.md`). In zad, `CLAUDE.md` is the canonical source — see the top of that file |
| §8.4 missing `CHANGELOG.md` | Create an empty Keep-a-Changelog-formatted file; do **not** hand-author entries (the release pipeline owns it) |
| §9 Makefile target missing | Add the missing target to `Makefile` and verify it runs end-to-end |
| §10.1/§10.3/§10.4 missing workflow | Create `.github/workflows/<file>.yml`; cross-reference the upstream oss-spec templates for the canonical shape |
| §10.3 floating or under-pinned toolchain | Edit the workflow to pin at or above the spec minimums |
| §11.1 missing `docs/` content | Create the topic file, then run `update-docs` |
| §11.2 website drift | Run `make website` and inspect `website/src/generated/`; follow up with `update-website` |
| §13.5 `prompts/<name>/` has no versioned file | Add `prompts/<name>/<major>_<minor>.md` with the required YAML front matter (`name`, `description`, `version`) and `## System` / `## User` sections (see `prompts/README.md`) |
| §15 missing issue / PR templates | Create the templates under `.github/ISSUE_TEMPLATE/` or `.github/PULL_REQUEST_TEMPLATE.md` |
| §19 raw print statement outside the central output module | Route the call through the project's logging / output helpers (`crates/zad/src/logging.rs` and the CLI render layer) |
| §20 inline `#[cfg(test)] mod { … }` block in `crates/*/src/` | Move the tests to `crates/<crate>/tests/<module>_test.rs` per the "Test conventions" section of `CLAUDE.md` |
| §20.2 test file stem does not end with `_test(s)` / `Test(s)` | Rename the file so the stem matches the regex `_?[Tt]ests?$` |
| §20.5 source file exceeds 1000 lines | **Preferred:** split the file by concern into sibling modules / helpers. **Common easy case:** if the file also has a §20 inline-test violation, extracting the test block to `tests/<stem>_test.rs` usually resolves both at once. **Escape hatch:** if the size is genuinely justified (generated code, cohesive state machine, third-party snapshot), add `oss-spec:allow-large-file: <reason>` in any comment within the file's first 20 lines — the reason must be non-empty |
| §21.2 `.claude/skills` is not a symlink | Replace it with `ln -s ../.agent/skills .claude/skills` |
| §21.3 SKILL.md missing front matter fields | Add `name:` / `description:` to the front matter |
| §21.4 missing `.last-updated` | Touch the file and record the current `HEAD`: `git rev-parse HEAD > .agent/skills/<skill>/.last-updated` |
| §21.5 missing required `update-*` skill | Create `.agent/skills/<skill>/SKILL.md` (+ `.last-updated`); register it in `maintenance/SKILL.md` |
| §21.6 `maintenance` skill registry row missing | Add the row in `maintenance/SKILL.md`, alphabetical, with a run-order slot |

## Update checklist

- [ ] Read the baseline from `.last-updated` and diff `OSS_SPEC.md`
- [ ] Run `oss-spec validate --no-ai --path .` and record every structural violation. If the binary is unavailable, run the bash fallback (`curl … | bash -s -- .`) — see the Nonbinary fallback note above
- [ ] Run `oss-spec validate --path .` and record every AI finding worth acting on (the bash fallback emits the same checklist as a manual prompt at the end of its run)
- [ ] Walk the mapping table and fix each violation at its source
- [ ] If a fix would require changing `OSS_SPEC.md` itself rather than the repo, stop — that work belongs upstream in the oss-spec repo, not here
- [ ] Re-run the validator — it must report zero structural violations (binary preferred; bash fallback if unavailable)
- [ ] Run `make fmt`, `make lint`, `make test` (skip these only if the toolchain is genuinely unavailable, and call out the unverified gap in the PR description)
- [ ] If the bash fallback rewrote `OSS_SPEC.md`, review and stage the diff (or revert it) deliberately — do not let an automatic spec bump slip into the commit unnoticed
- [ ] Write the new baseline:

      git rev-parse HEAD > .agent/skills/sync-oss-spec/.last-updated

## Verification

1. `oss-spec validate --no-ai --path .` prints a clean report. If the binary is unavailable, the bash fallback exits 0 instead.
2. `make test` passes. Skip only if the toolchain is genuinely unavailable; in that case call out the unverified gap in the PR description.
3. Every violation present before this run has a matching edit in the diff — no violations were silenced by editing `OSS_SPEC.md` itself or by working around the validator.
4. `.last-updated` was rewritten with the current `HEAD`.

## Skill self-improvement

After a run, extend this file:

1. **Grow the mapping table** whenever a new §X.Y section starts producing violations that the table does not yet cover.
2. **Record fix recipes** (exact commands or edit patterns) for violations that required more than a one-line change.
3. **Flag recurring drift** — if the same violation keeps coming back, either a CI check or a different skill's mapping table is missing a row. Fix the upstream cause, not just the symptom.
4. **Commit the skill edit** alongside the repo fixes so the knowledge compounds.
