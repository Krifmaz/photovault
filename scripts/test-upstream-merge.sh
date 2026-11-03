#!/usr/bin/env bash
# Exercise scripts/upstream-merge.sh against throwaway repositories:
# first import, clean update, per-directory conflict report (exit 1),
# --abort-on-conflict, and the up-to-date no-op. Touches nothing outside a temp dir.
set -euo pipefail

script="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/upstream-merge.sh"
tmp="$(mktemp -d)"
trap 'rm -rf -- "$tmp"' EXIT
export GIT_CONFIG_GLOBAL="$tmp/gitconfig" GIT_CONFIG_NOSYSTEM=1
git config --global user.name "Test"
git config --global user.email "test@example.invalid"
git config --global init.defaultBranch main
git config --global protocol.file.allow always

fail() {
    echo "FAIL: $1" >&2
    exit 1
}
commit_all() { git add -A && git commit --quiet -m "$1"; }

# Fake upstream: the three app dirs plus a server/ that must never be imported.
up="$tmp/ente"
mkdir -p "$up"/{mobile,web/apps,desktop,server}
git -C "$up" init --quiet
echo "mobile v1" >"$up/mobile/app.dart"
printf 'line1\nline2\nline3\n' >"$up/web/apps/page.ts"
echo "desktop v1" >"$up/desktop/main.ts"
echo "server" >"$up/server/main.go"
(cd "$up" && commit_all "upstream 1")

# Our fork, with its own files and placeholder dirs.
fork="$tmp/photovault"
mkdir -p "$fork"/{server,web}
git -C "$fork" init --quiet
echo "ours" >"$fork/README.md"
echo "rust" >"$fork/server/main.rs"
echo ".upstream/" >"$fork/.gitignore"
echo "placeholder" >"$fork/web/NOTES.md"
(cd "$fork" && commit_all "fork init")
cd "$fork"

run() {
    set +e
    "$script" --repo "file://$up" "$@" >"$tmp/out" 2>&1
    rc=$?
    set -e
}

# 1. First import.
run
[[ $rc -eq 0 ]] || { cat "$tmp/out"; fail "first import exit $rc"; }
[[ -f mobile/app.dart && -f web/apps/page.ts && -f desktop/main.ts ]] || fail "dirs not imported"
[[ ! -e server/main.go ]] || fail "upstream server/ leaked into the fork"
[[ -f server/main.rs && -f web/NOTES.md ]] || fail "fork files lost"
grep -q "commit = \"$(git -C "$up" rev-parse HEAD)\"" UPSTREAM.lock || fail "lock not written"
[[ "$(git ls-tree --name-only -r upstream/ente | sort | tr '\n' ' ')" == "desktop/main.ts mobile/app.dart web/apps/page.ts " ]] ||
    fail "snapshot branch holds more than the tracked dirs"
echo "ok: first import"

# 2. Local change + non-overlapping upstream change merge cleanly.
printf 'line1\nline2 (ours)\nline3\n' >web/apps/page.ts
commit_all "fork: patch web"
echo "mobile v2" >"$up/mobile/app.dart"
git -C "$up" rm --quiet desktop/main.ts
mkdir -p "$up/desktop" && echo "desktop v2" >"$up/desktop/new.ts"
echo "server v2" >"$up/server/main.go"
(cd "$up" && commit_all "upstream 2")
run
[[ $rc -eq 0 ]] || { cat "$tmp/out"; fail "clean update exit $rc"; }
[[ "$(cat mobile/app.dart)" == "mobile v2" ]] || fail "update not applied"
[[ ! -e desktop/main.ts && -f desktop/new.ts ]] || fail "upstream delete/add not applied"
grep -q "line2 (ours)" web/apps/page.ts || fail "fork patch lost"
[[ "$(git rev-list --count upstream/ente)" == 2 ]] || fail "snapshot history not linear"
echo "ok: clean update"

# 3. Up to date: no-op.
before="$(git rev-parse HEAD)"
run
[[ $rc -eq 0 && "$(git rev-parse HEAD)" == "$before" ]] || fail "up-to-date run changed HEAD"
echo "ok: up to date"

# 4. Conflict in web/ only: exit 1, report grouped by directory.
printf 'line1\nline2 (upstream)\nline3\n' >"$up/web/apps/page.ts"
echo "mobile v3" >"$up/mobile/app.dart"
(cd "$up" && commit_all "upstream 3")
run --abort-on-conflict
[[ $rc -eq 1 ]] || { cat "$tmp/out"; fail "conflict exit $rc (want 1)"; }
grep -q "web/: 1 file" "$tmp/out" || { cat "$tmp/out"; fail "conflict report missing web/"; }
grep -q "web/apps/page.ts" "$tmp/out" || fail "conflict report missing file"
! grep -q "mobile/:" "$tmp/out" || fail "mobile reported as conflicting"
[[ "$(git rev-parse HEAD)" == "$before" && -z "$(git status --porcelain --untracked-files=no)" ]] ||
    fail "--abort-on-conflict left changes"
echo "ok: conflict report + abort"

# 5. Same conflict, left in progress for resolution, then committed.
run
[[ $rc -eq 1 ]] || fail "second conflict run exit $rc"
git rev-parse --verify --quiet MERGE_HEAD >/dev/null || fail "merge not left in progress"
grep -q "commit = \"$(git -C "$up" rev-parse HEAD)\"" UPSTREAM.lock || fail "lock not staged during conflict"
printf 'line1\nline2 (both)\nline3\n' >web/apps/page.ts
git add web/apps/page.ts
git commit --quiet --no-edit
[[ "$(cat mobile/app.dart)" == "mobile v3" ]] || fail "non-conflicting update lost"
run
[[ $rc -eq 0 ]] || fail "after resolution not up to date"
echo "ok: resolve and commit"

# 6. Bad input is refused before touching git.
run --ref '--upload-pack=evil'
[[ $rc -eq 2 ]] || fail "option-like ref accepted"
run --dirs '../etc'
[[ $rc -eq 2 ]] || fail "path-like dir accepted"
echo "ok: input validation"

echo "All upstream-merge tests passed."
