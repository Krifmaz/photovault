#!/usr/bin/env bash
# Merge Ente's apps (mobile/, web/, desktop/ and the shared rust/ of ente-io/ente) into this repo.
#
# Mechanism (see UPSTREAM.md):
#   1. Shallow-fetch the requested upstream commit into a cache repo
#      (.upstream/ente.git, git-ignored).
#   2. Build a synthetic commit there whose tree holds ONLY the tracked
#      directories, with the previous synthetic commit as parent, and fetch it
#      into this repo as branch `upstream/ente`. That branch is a linear history
#      of upstream snapshots and is the merge base for the next import.
#   3. Merge `upstream/ente` into the current branch and record the upstream
#      commit in UPSTREAM.lock.
#
# Exit status: 0 merged (or already up to date), 1 merge conflicts (merge left
# in progress, report printed per directory), 2 usage or precondition error.
#
# Usage: scripts/upstream-merge.sh [--ref REF] [--repo URL] [--dirs "mobile web desktop rust"]
#                                  [--no-merge] [--abort-on-conflict]
set -euo pipefail

repo_url="https://github.com/ente-io/ente"
ref="main"
dirs="mobile web desktop rust"
do_merge=1
abort_on_conflict=0
branch="upstream/ente"
lock_file="UPSTREAM.lock"

die() {
    printf 'upstream-merge: %s\n' "$1" >&2
    exit 2
}

usage() {
    sed -n '2,19p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --ref) [[ $# -ge 2 ]] || usage; ref="$2"; shift 2 ;;
        --repo) [[ $# -ge 2 ]] || usage; repo_url="$2"; shift 2 ;;
        --dirs) [[ $# -ge 2 ]] || usage; dirs="$2"; shift 2 ;;
        --no-merge) do_merge=0; shift ;;
        --abort-on-conflict) abort_on_conflict=1; shift ;;
        -h | --help) usage ;;
        *) die "unknown argument: $1 (see --help)" ;;
    esac
done

# Validate everything that reaches git, so nothing can be read as an option or a path trick.
[[ "$ref" =~ ^[A-Za-z0-9][A-Za-z0-9._/-]{0,200}$ && "$ref" != *..* ]] || die "invalid --ref: $ref"
[[ "$repo_url" =~ ^(https://|file://|/)[A-Za-z0-9._~:/@%+-]+$ ]] || die "invalid --repo: $repo_url"
read -r -a dir_list <<<"$dirs"
[[ ${#dir_list[@]} -gt 0 ]] || die "--dirs is empty"
for d in "${dir_list[@]}"; do
    [[ "$d" =~ ^[A-Za-z0-9][A-Za-z0-9_-]*$ ]] || die "invalid directory name: $d"
done

top="$(git rev-parse --show-toplevel 2>/dev/null)" || die "not inside a git repository"
cd -- "$top"
git rev-parse --verify --quiet HEAD >/dev/null || die "repository has no commits"
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    die "working tree has uncommitted changes; commit or stash them first"
fi
if git rev-parse --verify --quiet MERGE_HEAD >/dev/null; then
    die "a merge is already in progress; finish or abort it first"
fi

cache="$top/.upstream/ente.git"
if [[ ! -d "$cache" ]]; then
    mkdir -p -- "$top/.upstream"
    git init --quiet --bare -- "$cache"
fi
ucache() { git --git-dir="$cache" "$@"; }

echo "Fetching $ref from $repo_url ..."
ucache fetch --quiet --depth 1 --no-tags -- "$repo_url" "$ref" ||
    die "fetch failed for $ref from $repo_url"
upstream_sha="$(ucache rev-parse --verify 'FETCH_HEAD^{commit}')"
upstream_date="$(ucache log -1 --format=%cI "$upstream_sha")"
echo "Upstream commit: $upstream_sha ($upstream_date)"

prev=""
if git rev-parse --verify --quiet "refs/heads/$branch" >/dev/null; then
    prev="$(git rev-parse "refs/heads/$branch")"
    prev_upstream="$(git log -1 --format=%B "$prev" | sed -n 's/^Upstream-Commit: //p' | head -n1)"
    if [[ "$prev_upstream" == "$upstream_sha" ]]; then
        echo "$branch already at $upstream_sha"
        if [[ $do_merge -eq 1 ]] && git merge-base --is-ancestor "$prev" HEAD; then
            echo "Already merged; nothing to do."
            exit 0
        fi
    else
        # The cache needs the previous snapshot to use it as parent.
        ucache fetch --quiet --no-tags -- "$top" "refs/heads/$branch:refs/pv/prev"
    fi
fi

if [[ -z "$prev" || "${prev_upstream:-}" != "$upstream_sha" ]]; then
    # Synthetic snapshot: only the tracked directories, nothing else from upstream.
    present=()
    entries=""
    for d in "${dir_list[@]}"; do
        if sub="$(ucache rev-parse --verify --quiet "$upstream_sha:$d")" &&
            [[ "$(ucache cat-file -t "$sub")" == tree ]]; then
            entries+="$(printf '040000 tree %s\t%s' "$sub" "$d")"$'\n'
            present+=("$d")
        else
            echo "warning: upstream has no $d/ at $upstream_sha" >&2
        fi
    done
    [[ ${#present[@]} -gt 0 ]] || die "none of the directories exist upstream"
    tree="$(printf '%s' "$entries" | ucache mktree)"
    msg="$(printf 'Upstream snapshot of %s at %s\n\nUpstream-Repo: %s\nUpstream-Commit: %s\nUpstream-Date: %s\n' \
        "${present[*]}" "${upstream_sha:0:12}" "$repo_url" "$upstream_sha" "$upstream_date")"
    parent_args=()
    [[ -n "$prev" ]] && parent_args=(-p "$prev")
    snapshot="$(GIT_AUTHOR_NAME="Photovault upstream import" GIT_AUTHOR_EMAIL="upstream@photovault.invalid" \
        GIT_COMMITTER_NAME="Photovault upstream import" GIT_COMMITTER_EMAIL="upstream@photovault.invalid" \
        ucache commit-tree "$tree" "${parent_args[@]}" -m "$msg")"
    ucache update-ref "refs/pv/snapshot" "$snapshot"
    git fetch --quiet --no-tags -- "$cache" "refs/pv/snapshot:refs/heads/$branch"
    echo "Updated $branch -> ${snapshot:0:12}"
fi

if [[ $do_merge -eq 0 ]]; then
    echo "--no-merge: branch $branch updated; merge it with: git merge $branch"
    exit 0
fi

merge_args=(--no-ff --no-commit)
if ! git merge-base HEAD "refs/heads/$branch" >/dev/null 2>&1; then
    echo "First import: merging unrelated history."
    merge_args+=(--allow-unrelated-histories)
fi

set +e
git merge "${merge_args[@]}" -m "Merge upstream ente ${upstream_sha:0:12} (${dirs})" "refs/heads/$branch" >/dev/null 2>&1
merge_status=$?
set -e

write_lock() {
    {
        echo "# Managed by scripts/upstream-merge.sh; do not edit by hand."
        echo "repo = \"$repo_url\""
        echo "ref = \"$ref\""
        echo "commit = \"$upstream_sha\""
        echo "commit_date = \"$upstream_date\""
        echo "dirs = \"$dirs\""
    } >"$lock_file"
    git add -- "$lock_file"
}

mapfile -t conflicts < <(git diff --name-only --diff-filter=U)
if [[ $merge_status -ne 0 && ${#conflicts[@]} -eq 0 ]]; then
    git merge --abort >/dev/null 2>&1 || true
    die "git merge failed without conflicts (see 'git merge $branch')"
fi

if [[ ${#conflicts[@]} -gt 0 ]]; then
    echo
    echo "CONFLICTS merging upstream ${upstream_sha:0:12}:"
    declare -A per_dir=()
    for f in "${conflicts[@]}"; do
        top_dir="${f%%/*}"
        per_dir["$top_dir"]=$((${per_dir["$top_dir"]:-0} + 1))
    done
    for d in "${!per_dir[@]}"; do
        printf '  %s/: %d file(s)\n' "$d" "${per_dir[$d]}"
        for f in "${conflicts[@]}"; do
            [[ "${f%%/*}" == "$d" ]] && printf '      %s\n' "$f"
        done
    done
    if [[ $abort_on_conflict -eq 1 ]]; then
        git merge --abort
        echo "Merge aborted (--abort-on-conflict). $branch still points at the new snapshot."
    else
        write_lock
        echo
        echo "Resolve the files above, then: git add <files> && git commit"
        echo "Or abandon with: git merge --abort"
    fi
    exit 1
fi

write_lock
git commit --quiet --no-edit
echo "Merged upstream ${upstream_sha:0:12} into $(git rev-parse --abbrev-ref HEAD) ($(git rev-parse --short HEAD))."
