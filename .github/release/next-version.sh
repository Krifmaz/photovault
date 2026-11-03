#!/usr/bin/env bash
# Prints the next release tag for the current checkout.
#
#   next-version.sh [patch|minor|major] [pre-release label]
#
# Stable (no label): the workspace version in Cargo.toml is the floor. If it
# is newer than every stable tag it is used as is (so bumping Cargo.toml to
# 0.2.0 releases v0.2.0); otherwise the newest stable tag is bumped.
#
# Pre-release (label, e.g. alpha): v<next stable>-<label>.<N>, where N counts
# up from 1 for each pre-release of that upcoming version.
set -euo pipefail

bump="${1:-patch}"
label="${2:-}"
base=$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\([0-9.]*\)".*/\1/p}' Cargo.toml | head -n1)
[[ "$base" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "cannot read workspace version from Cargo.toml" >&2; exit 1; }

latest=$(git tag -l 'v*' | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sed 's/^v//' | sort -V | tail -n1 || true)

if [[ -z "$latest" ]] || { [[ "$latest" != "$base" ]] && [[ "$(printf '%s\n%s\n' "$latest" "$base" | sort -V | tail -n1)" == "$base" ]]; }; then
  next="$base"
else
  IFS=. read -r major minor patch <<<"$latest"
  case "$bump" in
    major) next="$((major + 1)).0.0" ;;
    minor) next="$major.$((minor + 1)).0" ;;
    patch) next="$major.$minor.$((patch + 1))" ;;
    *) echo "unknown bump: $bump" >&2; exit 1 ;;
  esac
fi

if [[ -z "$label" ]]; then
  echo "v$next"
  exit 0
fi
[[ "$label" =~ ^[a-z]+$ ]] || { echo "bad pre-release label: $label" >&2; exit 1; }
n=$(git tag -l "v$next-$label.*" | sed "s/^v$next-$label\.//" | grep -E '^[0-9]+$' | sort -n | tail -n1 || true)
echo "v$next-$label.$(( ${n:-0} + 1 ))"
