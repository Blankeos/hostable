#!/usr/bin/env bash

# Release from your own machine: bump the workspace version, regenerate the
# changelog, commit, tag and push. The tag push triggers .github/workflows/release.yml.

set -euo pipefail

if [ -n "$(git status --porcelain)" ]; then
    echo "Please commit all changes before bumping the version."
    exit 1
fi

branch="$(git branch --show-current)"
if [[ "$branch" != "main" ]]; then
    echo "Releases must be tagged from main (currently on '${branch:-detached HEAD}')."
    echo "  git checkout main && git pull && just tag"
    exit 1
fi

echo "Fetching origin/main..."
git fetch --quiet origin main

if ! git merge-base --is-ancestor origin/main HEAD; then
    echo "local main has diverged from origin/main. Pull/rebase first."
    exit 1
fi

# The version lives once, in [workspace.package].
CURRENT=$(awk '/^\[workspace.package\]/{f=1;next} /^\[/{f=0} f && /^version *=/{gsub(/[" ]/,"",$0); sub(/version=/,"",$0); print; exit}' Cargo.toml)
if [ -z "$CURRENT" ]; then
    echo "Could not read [workspace.package] version from Cargo.toml"
    exit 1
fi
echo "What kind of change is this for hostable? (current version is $CURRENT) [patch, minor, major] >"

BUMP="${1:-}"
if [ -z "$BUMP" ]; then
    read -r BUMP
fi

case "$BUMP" in
    patch) NEW=$(echo "$CURRENT" | awk -F. '{$NF+=1; OFS="."; print $1,$2,$3}') ;;
    minor) NEW=$(echo "$CURRENT" | awk -F. '{$(NF-1)+=1; $NF=0; OFS="."; print $1,$2,$3}') ;;
    major) NEW=$(echo "$CURRENT" | awk -F. '{$1+=1; $2=0; $3=0; OFS="."; print $1,$2,$3}') ;;
    *) echo "Please specify patch, minor, or major"; exit 1 ;;
esac

echo "Would tag and push hostable $CURRENT -> $NEW"
read -p "Proceed? [Y/n] " -r CONFIRM
CONFIRM=${CONFIRM:-y}
if [[ ! "$CONFIRM" =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 0
fi

echo "Checking the build before tagging..."
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked

echo "Updating Cargo.toml to version ${NEW}"
sed -i.bak "/^\[workspace.package\]/,/^\[/ s/^version *= *\"[^\"]*\"/version = \"${NEW}\"/" Cargo.toml
rm Cargo.toml.bak

echo "Updating Cargo.lock..."
cargo update --workspace --quiet

if [ -f cliff.toml ] && command -v git-cliff >/dev/null 2>&1; then
    echo "Regenerating CHANGELOG.md..."
    git cliff --tag "v${NEW}" -o CHANGELOG.md
fi

echo "Committing version bump ${NEW}..."
git add .
git commit -m "release: hostable v${NEW}"

echo "Creating git tag v${NEW}"
git tag "v${NEW}"

echo "Pushing release commit and tag atomically..."
git push --atomic origin main "v${NEW}"
