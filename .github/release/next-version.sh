#!/usr/bin/env bash
# Prints the next release tag (vX.Y.Z) for the current checkout.
#
#   next-version.sh [patch|minor|major]
#
# The workspace version in Cargo.toml is the floor: if tag v<that> does not
# exist yet it is used as is, so bumping Cargo.toml to 0.2.0 releases v0.2.0.
# Otherwise the newest existing tag is bumped (patch by default).
set -euo pipefail

bump="${1:-patch}"
base=$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\([0-9.]*\)".*/\1/p}' Cargo.toml | head -n1)
[[ "$base" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "cannot read workspace version from Cargo.toml" >&2; exit 1; }

latest=$(git tag -l 'v[0-9]*.[0-9]*.[0-9]*' | sed 's/^v//' | sort -V | tail -n1)

# A Cargo.toml version newer than every tag is released as is.
if [[ -z "$latest" ]] || { [[ "$latest" != "$base" ]] && [[ "$(printf '%s\n%s\n' "$latest" "$base" | sort -V | tail -n1)" == "$base" ]]; }; then
  echo "v$base"
  exit 0
fi

IFS=. read -r major minor patch <<<"$latest"
case "$bump" in
  major) echo "v$((major + 1)).0.0" ;;
  minor) echo "v$major.$((minor + 1)).0" ;;
  patch) echo "v$major.$minor.$((patch + 1))" ;;
  *) echo "unknown bump: $bump" >&2; exit 1 ;;
esac
