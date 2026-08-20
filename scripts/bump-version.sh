#!/usr/bin/env bash
# Bump the xcode-mcp workspace version, commit, tag, and push.
# Triggers .github/workflows/release.yml which renders the Homebrew formula
# and pushes it to vexornp/homebrew-xcode-mcp.
#
# Usage: ./scripts/bump-version.sh 0.2.0
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "Usage: $0 <new-version>   e.g. $0 0.2.0" >&2
  exit 2
fi

NEW="$1"

# Validate semver (major.minor.patch, optional leading v, no pre-release for releases).
if ! [[ "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: '$NEW' is not a valid semver (expect major.minor.patch, e.g. 0.2.0)" >&2
  exit 1
fi

# Locate repo root.
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || {
  echo "error: not inside a git repository" >&2
  exit 1
}
cd "$REPO_ROOT"

# Ensure clean working tree.
if [ -n "$(git status --porcelain)" ]; then
  echo "error: working tree has uncommitted changes; commit or stash first" >&2
  git status --short >&2
  exit 1
fi

# Read current version.
CURRENT=$(grep -m1 '^version = "' Cargo.toml | sed -E 's/version = "([^"]+)".*/\1/')
if [ -z "$CURRENT" ]; then
  echo "error: could not read version from Cargo.toml" >&2
  exit 1
fi

if [ "$NEW" = "$CURRENT" ]; then
  echo "error: new version '$NEW' equals current version" >&2
  exit 1
fi

# Compare versions numerically — refuse downgrades.
higher() {
  # Returns 0 if $1 > $2, else 1.
  local a="$1" b="$2"
  local IFS=.
  local i ai bi
  read -ra A <<<"$a"
  read -ra B <<<"$b"
  for ((i = 0; i < ${#A[@]} || i < ${#B[@]}; i++)); do
    ai=${A[i]:-0}
    bi=${B[i]:-0}
    if ((ai > bi)); then return 0; fi
    if ((ai < bi)); then return 1; fi
  done
  return 1
}
if ! higher "$NEW" "$CURRENT"; then
  echo "error: new version '$NEW' is not greater than current '$CURRENT'" >&2
  exit 1
fi

# Ensure we're on main (releases from main).
BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [ "$BRANCH" != "main" ]; then
  echo "error: must be on 'main' (currently on '$BRANCH')" >&2
  exit 1
fi

# Ensure main is up to date with origin.
git fetch origin main --quiet
LOCAL=$(git rev-parse HEAD)
REMOTE=$(git rev-parse origin/main)
if [ "$LOCAL" != "$REMOTE" ]; then
  echo "error: local 'main' is not in sync with 'origin/main' (fetch/rebase first)" >&2
  exit 1
fi

echo "==> Bumping version $CURRENT -> $NEW"

# Edit Cargo.toml (workspace.package version is the single source of truth).
# Uses a temp file + mv so we never leave a half-edited Cargo.toml on failure.
TMP=$(mktemp)
sed -E "s/^version = \"[^\"]+\"/version = \"$NEW\"/" Cargo.toml > "$TMP"
mv "$TMP" Cargo.toml

# Refresh Cargo.lock without a full build.
echo "==> Refreshing Cargo.lock"
if ! cargo check --quiet 2>&1 | tail -5; then
  echo "error: cargo check failed, restoring Cargo.toml" >&2
  git checkout -- Cargo.toml
  exit 1
fi

# Show what we're about to commit.
echo "==> Diff"
git diff --stat

# Commit.
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to $NEW" --quiet
echo "==> Committed"

# Tag + push. Tag push triggers .github/workflows/release.yml.
TAG="v$NEW"
git tag "$TAG"
echo "==> Tagged $TAG"

echo "==> Pushing main + tag (this triggers the release workflow)"
git push origin main
git push origin "$TAG"

echo ""
echo "==> Done. Watch the workflow:"
echo "    https://github.com/vexornp/XcodeMcp/actions"
echo ""
echo "    When it completes, users can upgrade with:"
echo "    brew update && brew upgrade xcode-mcp"
