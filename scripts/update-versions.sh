#!/usr/bin/env bash
# Update version strings in language-specific manifests to match the given tag.
set -euo pipefail

tag="${1:?usage: update-versions.sh <tag>}"
ver="${tag#v}"

if [ -f Cargo.toml ]; then
  sed -i.bak -E "s/^version = \".*\"/version = \"${ver}\"/" Cargo.toml && rm Cargo.toml.bak
fi
# Keep Cargo.lock's entry for the workspace crate in sync with Cargo.toml so
# `cargo publish` does not regenerate the lockfile and fail on dirty working
# tree. We only touch the `version = "…"` line that immediately follows the
# `name = "<crate>"` line for the local crate.
if [ -f Cargo.toml ] && [ -f Cargo.lock ]; then
  crate="$(sed -n -E 's/^name = "([^"]+)"/\1/p' Cargo.toml | head -n1)"
  if [ -n "${crate}" ]; then
    awk -v crate="${crate}" -v ver="${ver}" '
      found && /^version = ".*"$/ {
        sub(/"[^"]*"/, "\"" ver "\"")
        found = 0
      }
      $0 == "name = \"" crate "\"" { found = 1 }
      { print }
    ' Cargo.lock > Cargo.lock.tmp && mv Cargo.lock.tmp Cargo.lock
  fi
fi
if [ -f package.json ]; then
  sed -i.bak -E "s/(\"version\": \")[^\"]*(\")/\1${ver}\2/" package.json && rm package.json.bak
fi
if [ -f pyproject.toml ]; then
  sed -i.bak -E "s/^version = \".*\"/version = \"${ver}\"/" pyproject.toml && rm pyproject.toml.bak
fi
