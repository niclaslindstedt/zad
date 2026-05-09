#!/usr/bin/env bash
# Update version strings in language-specific manifests to match the given tag.
#
# For the Cargo workspace this means rewriting the shared
# `[workspace.package].version` once at the workspace root — both
# `crates/zad/Cargo.toml` and `crates/zad-cli/Cargo.toml` inherit
# their version from there. The path dep `zad = { path = "../zad",
# version = "x.y.z" }` inside `crates/zad-cli/Cargo.toml` carries its
# own pinned version string and is rewritten too so the binary
# always depends on a matching library version.
set -euo pipefail

tag="${1:?usage: update-versions.sh <tag>}"
ver="${tag#v}"

# Workspace root manifest: rewrite `[workspace.package].version` only.
if [ -f Cargo.toml ]; then
  awk -v ver="${ver}" '
    /^\[workspace\.package\]/ { in_block = 1; print; next }
    /^\[/ && in_block { in_block = 0 }
    in_block && /^version = ".*"$/ { sub(/"[^"]*"/, "\"" ver "\""); print; next }
    { print }
  ' Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml
fi

# Member manifests: rewrite each crate's path-dep on `zad` so the
# binary always pins the matching library version.
if [ -f crates/zad-cli/Cargo.toml ]; then
  sed -i.bak -E "s|(zad = \{ path = \"\.\./zad\", version = \")[^\"]*(\" \})|\1${ver}\2|" crates/zad-cli/Cargo.toml \
    && rm crates/zad-cli/Cargo.toml.bak
fi

# Keep Cargo.lock's entries for our local crates in sync with the
# workspace version so `cargo publish` does not regenerate the
# lockfile and fail on a dirty working tree.
if [ -f Cargo.lock ]; then
  for crate in zad zad-cli; do
    awk -v crate="${crate}" -v ver="${ver}" '
      found && /^version = ".*"$/ {
        sub(/"[^"]*"/, "\"" ver "\"")
        found = 0
      }
      $0 == "name = \"" crate "\"" { found = 1 }
      { print }
    ' Cargo.lock > Cargo.lock.tmp && mv Cargo.lock.tmp Cargo.lock
  done
fi
if [ -f package.json ]; then
  sed -i.bak -E "s/(\"version\": \")[^\"]*(\")/\1${ver}\2/" package.json && rm package.json.bak
fi
if [ -f pyproject.toml ]; then
  sed -i.bak -E "s/^version = \".*\"/version = \"${ver}\"/" pyproject.toml && rm pyproject.toml.bak
fi
