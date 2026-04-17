#!/usr/bin/env bash
# Fail if any tool-specific agent-guidance file has been dereferenced
# into a regular file. Per OSS_SPEC.md §7.1 these must stay symlinks
# that point at AGENTS.md so the single-source-of-truth rule holds.
set -euo pipefail

paths=(
  CLAUDE.md
  .cursorrules
  .windsurfrules
  GEMINI.md
  .aider.conf.md
  .github/copilot-instructions.md
)

fail=0
for p in "${paths[@]}"; do
  if [ ! -e "$p" ]; then
    echo "error: $p is missing (must be a symlink to AGENTS.md)" >&2
    fail=1
    continue
  fi
  if [ ! -L "$p" ]; then
    echo "error: $p is a regular file; must be a symlink to AGENTS.md" >&2
    fail=1
    continue
  fi
  target=$(readlink "$p")
  case "$target" in
    AGENTS.md|../AGENTS.md) ;;
    *)
      echo "error: $p -> $target; expected AGENTS.md or ../AGENTS.md" >&2
      fail=1
      ;;
  esac
done

# .claude/skills must symlink to ../.agent/skills (§21.2).
if [ ! -L .claude/skills ]; then
  echo "error: .claude/skills must be a symlink to ../.agent/skills" >&2
  fail=1
else
  target=$(readlink .claude/skills)
  if [ "$target" != "../.agent/skills" ]; then
    echo "error: .claude/skills -> $target; expected ../.agent/skills" >&2
    fail=1
  fi
fi

exit "$fail"
