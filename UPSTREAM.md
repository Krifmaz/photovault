# Keeping the app forks up to date

`mobile/`, `web/` and `desktop/` are **light forks** of the directories of the
same names in [ente-io/ente](https://github.com/ente-io/ente). Photovault keeps
Ente's apps as they are; local changes there should stay small (branding,
defaults, a few hooks) so upstream improvements and security fixes merge
cleanly. The server does not depend on these forks: the stock apps already
work against it.

## How it works

`scripts/upstream-merge.sh` maintains a branch **`upstream/ente`** whose
commits are *snapshots* of only those three directories at a given upstream
commit:

```text
ente-io/ente main ── A ─── B ─── C            (full upstream history; never fetched in full)
                     ┆     ┆     ┆   snapshot = tree of mobile/ web/ desktop/ only
upstream/ente ────── a ─── b ─── c            (linear, one commit per import)
                      ╲     ╲     ╲
main ───────────────── M1 ── M2 ── M3         (regular merges into our history)
```

1. The requested upstream commit is fetched **shallowly** (`--depth 1`) into a
   private cache repository, `.upstream/ente.git` (git-ignored), so this
   repository never receives Ente's full history or its other directories.
2. A snapshot commit is built there from the three subtrees (`git mktree`)
   with the previous snapshot as its parent, and fetched into this repository
   as `upstream/ente`. Its message records `Upstream-Repo`, `Upstream-Commit`
   and `Upstream-Date`.
3. `upstream/ente` is merged into the current branch with a normal
   three-way merge. The previous snapshot is the merge base, so only
   upstream's changes since the last import are applied on top of ours.
4. `UPSTREAM.lock` records the imported upstream commit, its date, the ref
   and the directories, and is committed with the merge.

Because each snapshot contains only `mobile/`, `web/` and `desktop/`, a merge
can never touch `server/`, `docs/` or anything else of ours. Upstream
deletions and renames inside the three directories are applied like any
other change.

No `git subtree`, `git filter-repo` or other extra tools are needed: only git
and bash.

## Updating

```sh
git switch main
git switch -c upstream-sync
scripts/upstream-merge.sh                 # latest ente-io/ente main
scripts/upstream-merge.sh --ref v1.2.3    # or a tag, branch or commit id
```

Exit status:

| Status | Meaning |
| --- | --- |
| `0` | Merged and committed, or already up to date. |
| `1` | Conflicts. The merge is left in progress with `UPSTREAM.lock` staged and a report per directory, e.g. `web/: 2 file(s)` followed by the paths. Resolve, `git add`, `git commit` (or `git merge --abort`). |
| `2` | Usage or precondition error (dirty tree, merge in progress, bad argument, fetch failure). Nothing was changed. |

Options:

| Option | Effect |
| --- | --- |
| `--ref REF` | Upstream branch, tag or commit (default `main`). |
| `--repo URL` | Upstream repository (default `https://github.com/ente-io/ente`; `file://` for testing). |
| `--dirs "mobile web desktop"` | Directories to track. |
| `--no-merge` | Only update `upstream/ente`; merge it yourself later. |
| `--abort-on-conflict` | Report conflicts, then abort the merge (CI use). |

Then build and test the apps, open a pull request from `upstream-sync`, and
merge it.

### The first import

The first run merges `upstream/ente` with `--allow-unrelated-histories`.
Placeholder files we committed before the import (such as
`mobile/README.md`) conflict with upstream's files of the same name; resolve
those in favour of upstream (`git checkout --theirs mobile/README.md`).

## Rules for fork changes

* Keep changes minimal and grouped; prefer configuration over edits.
* Never change the apps' cryptography or wire protocol: the server must
  keep working with the **unmodified** upstream apps.
* Note each deliberate divergence in a `PHOTOVAULT.md` file in the app's
  directory so conflicts are easy to judge.

## Testing the mechanism

`scripts/test-upstream-merge.sh` builds throwaway upstream and fork
repositories and checks the first import, a clean update (including upstream
deletions), the up-to-date no-op, the per-directory conflict report and exit
status, conflict resolution, and input validation. It touches nothing
outside a temporary directory.
