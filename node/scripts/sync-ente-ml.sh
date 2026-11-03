#!/usr/bin/env bash
# Vendor Ente's Rust ML crates into node/vendor/ente.
#
# Usage: node/scripts/sync-ente-ml.sh <path-to-ente-checkout>
#
# Copies rust/crates/{ml,image,assets} from an ente-io/ente checkout, rewrites
# every `*.workspace = true` key to the concrete value from Ente's workspace
# Cargo.toml (so the crates build outside Ente's workspace), records the
# upstream commit in node/vendor/ente/UPSTREAM_REV, then re-applies
# Photovault's patches from node/vendor/ente/patches/*.patch in order.
# Patches are listed and explained in node/vendor/ente/PATCHES.md.
#
# The script never evaluates file contents; the only input is the checkout
# path, which is passed to commands as a quoted argument.
#
# Based on Ente (ente.com). SPDX-License-Identifier: AGPL-3.0-only

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 <path-to-ente-checkout>" >&2
    exit 2
fi

ente_root=$1
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
vendor_dir=$(cd -- "$script_dir/.." && pwd)/vendor/ente
rust_root=$ente_root/rust
crates=(ml image assets)

if [[ ! -f "$rust_root/Cargo.toml" ]]; then
    echo "error: $rust_root/Cargo.toml not found; pass the root of an ente checkout" >&2
    exit 1
fi

rev=$(git -C "$ente_root" rev-parse HEAD)
rev_date=$(git -C "$ente_root" show -s --format=%cs HEAD)

mkdir -p "$vendor_dir"
for crate in "${crates[@]}"; do
    src=$rust_root/crates/$crate
    dst=$vendor_dir/$crate
    if [[ ! -f "$src/Cargo.toml" ]]; then
        echo "error: missing crate $src" >&2
        exit 1
    fi
    rm -rf -- "$dst"
    mkdir -p -- "$dst"
    # Copy tracked sources only (no target/ or local build output).
    (cd -- "$src" && git ls-files -z) | while IFS= read -r -d '' f; do
        mkdir -p -- "$dst/$(dirname -- "$f")"
        cp -p -- "$src/$f" "$dst/$f"
    done
done

# Rewrite workspace inheritance into concrete values.
python3 -I - "$rust_root/Cargo.toml" "$vendor_dir" "${crates[@]}" <<'PY'
import re, sys, tomllib

ws_path, vendor_dir, *crates = sys.argv[1:]
with open(ws_path, "rb") as f:
    ws = tomllib.load(f)["workspace"]
ws_deps = ws.get("dependencies", {})
ws_pkg = ws.get("package", {})
# Ente crates that are vendored side by side.
local = {"ente-" + c: "../" + c for c in crates}


def render(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, str):
        return '"' + v.replace("\\", "\\\\").replace('"', '\\"') + '"'
    if isinstance(v, list):
        return "[" + ", ".join(render(x) for x in v) + "]"
    if isinstance(v, dict):
        return "{ " + ", ".join(f"{k} = {render(x)}" for k, x in v.items()) + " }"
    raise TypeError(type(v))


def resolve(name, member):
    base = ws_deps.get(name)
    if base is None:
        sys.exit(f"error: {name} is not in the workspace dependencies")
    base = {"version": base} if isinstance(base, str) else dict(base)
    if name in local:
        base = {"path": local[name]}
    elif "path" in base:
        sys.exit(f"error: {name} is a path dependency that is not vendored")
    out = dict(base)
    for k, v in member.items():
        if k == "workspace":
            continue
        if k == "features":
            feats = list(out.get("features", []))
            feats += [x for x in v if x not in feats]
            out["features"] = feats
        else:
            out[k] = v
    if list(out) == ["version"]:
        return render(out["version"])
    return render(out)


def find_close(text, start):
    depth, i, in_str = 0, start, False
    while i < len(text):
        c = text[i]
        if in_str:
            if c == "\\":
                i += 1
            elif c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    sys.exit("error: unbalanced inline table")


for crate in crates:
    path = f"{vendor_dir}/{crate}/Cargo.toml"
    text = open(path, encoding="utf-8").read()

    # [package] fields: `edition.workspace = true`.
    def pkg(m):
        key = m.group(1)
        if key not in ws_pkg:
            sys.exit(f"error: workspace.package.{key} missing")
        return f"{key} = {render(ws_pkg[key])}"

    text = re.sub(r"(?m)^(edition|version|license|rust-version)\.workspace\s*=\s*true", pkg, text)

    # The `[lints]` table: Ente's lint set is for Ente's workspace.
    text = re.sub(
        r"(?m)^\[lints\]\s*\nworkspace\s*=\s*true\s*\n?",
        "# [lints] inherited from Ente's workspace: removed by sync-ente-ml.sh because\n"
        "# Ente's workspace lints do not apply outside Ente's workspace.\n",
        text,
    )

    # `name.workspace = true`
    text = re.sub(
        r"(?m)^([A-Za-z0-9_-]+)\.workspace\s*=\s*true",
        lambda m: f"{m.group(1)} = {resolve(m.group(1), {})}",
        text,
    )

    # `name = { workspace = true, ... }`, possibly spanning lines.
    out, pos = [], 0
    pat = re.compile(r"(?m)^([A-Za-z0-9_-]+)\s*=\s*\{")
    while True:
        m = pat.search(text, pos)
        if not m:
            out.append(text[pos:])
            break
        brace = m.end() - 1
        end = find_close(text, brace)
        frag = text[brace : end + 1]
        table = tomllib.loads("x = " + frag)["x"]
        out.append(text[pos : m.start()])
        if table.get("workspace") is True:
            out.append(f"{m.group(1)} = {resolve(m.group(1), table)}")
        else:
            out.append(text[m.start() : end + 1])
        pos = end + 1
    text = "".join(out)

    if re.search(r"workspace\s*=\s*true", text):
        sys.exit(f"error: unresolved workspace inheritance left in {path}")
    tomllib.loads(text)
    open(path, "w", encoding="utf-8").write(text)
PY

printf '%s\n%s\n' "$rev" "$rev_date" >"$vendor_dir/UPSTREAM_REV"

# Re-apply Photovault patches.
shopt -s nullglob
failed=0
for p in "$vendor_dir"/patches/*.patch; do
    if patch -d "$vendor_dir" -p1 --forward --batch --silent <"$p"; then
        echo "applied $(basename -- "$p")"
    else
        echo "CONFLICT: $(basename -- "$p") did not apply cleanly; see PATCHES.md" >&2
        failed=1
    fi
done
find "$vendor_dir" \( -name '*.orig' -o -name '*.rej' \) -print

echo "vendored ente rust/crates/{${crates[*]// /,}} at $rev ($rev_date)"
exit "$failed"
