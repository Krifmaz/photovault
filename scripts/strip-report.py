#!/usr/bin/env python3
# PHOTOVAULT: strip report (new file).
#
# Greps the Photovault app sources (and optionally a built APK and the web
# static export) for Ente company branding, Ente hosts and the services the
# fork strips (billing, referrals, crash reporting, store links). Every hit
# must be explained by an entry of scripts/strip-allowlist.toml; the report
# lists each hit with its explanation and an "Unexplained" section.
#
#   python3 -I scripts/strip-report.py                      # source, write docs/src/strip-report.md
#   python3 -I scripts/strip-report.py --check              # same, exit 1 on unexplained hits
#   python3 -I scripts/strip-report.py --check --apk app.apk --web-out web/apps/photos/out ...
#   python3 -I scripts/strip-report.py --no-source --apk app.apk --output report.md
#
# Python 3.11+ standard library only. Run it with -I: the APK and the web
# export are build outputs and are only read as data (zip members are read in
# memory, never extracted or executed).
#
# What is scanned (source_roots() below, minus the [[exclude]] paths of the
# allowlist):
#   * mobile/apps/photos and every mobile path dependency it pulls in
#     (followed transitively through pubspec.yaml `path:` entries),
#   * web/apps/{photos,albums,accounts,cast,embed,memories,share}, web/packages,
#   * desktop/src, desktop/build (metadata), desktop/package.json and
#     desktop/electron-builder.yml,
#   * src/ of the Rust crates built into the apps (RUST_BINDINGS and their
#     path dependencies, followed through Cargo.toml; inline
#     `#[cfg(test)] mod` blocks are skipped),
#   * .arb and locale translation.json files (string values only).
#
# Contexts. Comments are not scanned (they are neither compiled into the
# apps nor shown). Terms marked "code" match anywhere else on a line
# (identifiers, string literals, markup). Terms marked "visible" (the plain
# word "Ente", duck, referral, subscription, ...) match only where text can
# reach a user: string literals, markup text/attributes, JSX text, and the
# values of translation files.
#
# --apk: classes*.dex, resources.arsc, AndroidManifest.xml and res/*.xml
# (printable ASCII and UTF-16 strings), lib/*/libapp.so (the Dart snapshot)
# and Photovault's own Rust libraries (lib/*/libente*.so, librust*.so), with the "code"
# terms (resources: all terms); text assets (flutter_assets etc.) like source.
# For binaries the "line" is the index of the extracted string.
# --web-out: JS bundles (term matches with the string around them, and the
# inlined translation files with their keys), HTML, JSON and the like.

from __future__ import annotations

import argparse
import fnmatch
import json
import os
import re
import sys
import tomllib
import zipfile
from dataclasses import dataclass

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_ALLOWLIST = os.path.join(REPO, "scripts", "strip-allowlist.toml")
DEFAULT_OUTPUT = os.path.join(REPO, "docs", "src", "strip-report.md")

# --------------------------------------------------------------------------
# Terms

# (id, regex, context). "code": anywhere outside comments; "visible": only in
# user-visible text (string literals, markup, translation values).
_HOST_PRE = r"(?<![\w-])(?:[\w-]+\.)*"
TERMS: list[tuple[str, re.Pattern[str], str]] = [
    ("support@ente", re.compile(r"support@ente\.(?:com|io)", re.I), "code"),
    ("ente.com", re.compile(_HOST_PRE + r"ente\.com(?-i:(?![a-z0-9-]|\.[a-z]))", re.I), "code"),
    ("ente.io", re.compile(_HOST_PRE + r"ente\.io(?-i:(?![a-z0-9-]|\.[a-z]))(?!/local_auth)", re.I), "code"),
    ("ente.sh", re.compile(_HOST_PRE + r"ente\.sh(?-i:(?![a-z0-9-]|\.[a-z]))", re.I), "code"),
    ("github.com/ente-io", re.compile(r"github\.com/ente-io\b", re.I), "code"),
    ("sentry", re.compile(r"(?<![a-z])sentry", re.I), "code"),
    ("firebase", re.compile(r"(?<![a-z])firebase", re.I), "code"),
    ("play.google", re.compile(r"play\.google", re.I), "code"),
    ("apps.apple", re.compile(r"apps\.apple", re.I), "code"),
    # The word "Ente" (not part of a host, an identifier like io.ente.x or
    # ente_ui, or an e-mail address).
    (
        "ente",
        re.compile(r"(?<![\w.@/\\-])ente(?![\w-]|\.(?:com|io|sh)\b|\.\w)", re.I),
        "visible",
    ),
    ("duck", re.compile(r"\bduck(?:y|ies|s)?\b", re.I), "visible"),
    ("mascot", re.compile(r"\bmascots?\b", re.I), "visible"),
    ("referral", re.compile(r"\breferrals?\b", re.I), "visible"),
    ("merch", re.compile(r"\bmerch(?:andise)?\b", re.I), "visible"),
    ("rate us", re.compile(r"\brate\s+(?:us|the\s+app)\b", re.I), "visible"),
    ("subscription", re.compile(r"\bsubscriptions?\b", re.I), "visible"),
]
TERM_IDS = [t[0] for t in TERMS]
# In native code string literals sit back to back ("https://x.ente.comfoo"),
# so in binaries a host may be followed by anything.
_HOST_END = "(?-i:(?![a-z0-9-]|\\.[a-z]))"
BINARY_TERM_RX = [(t, re.compile(rx.pattern.replace(_HOST_END, ""), rx.flags), ctx)
                  for t, rx, ctx in TERMS]

# --------------------------------------------------------------------------
# What to scan

WEB_APPS = ["photos", "albums", "accounts", "cast", "embed", "memories", "share"]

CLIKE = {".dart", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".kt", ".kts",
         ".java", ".swift", ".m", ".mm", ".h", ".c", ".cc", ".cpp", ".rs",
         ".gradle", ".css", ".scss"}
MARKUP = {".xml", ".html", ".plist", ".svg", ".storyboard", ".xib",
          ".entitlements"}
HASHCOMMENT = {".yaml", ".yml", ".properties", ".pro", ".toml", ".cmake",
               ".desktop", ".sh"}
JSONLIKE = {".json", ".webmanifest"}
TEXTLIKE = {".txt"}
SCANNED_EXT = CLIKE | MARKUP | HASHCOMMENT | JSONLIKE | TEXTLIKE | {".arb"}
# Files without one of the extensions above that are still read as text.
SCANNED_NAMES = {"CMakeLists.txt", "Podfile", "Info.plist", "_headers",
                 "_redirects"}


def _pubspec_path_deps(pubspec: str) -> list[str]:
    """`path:` dependencies of a pubspec.yaml (dependencies and overrides)."""
    out = []
    try:
        with open(pubspec, encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return out
    section = None
    for line in text.splitlines():
        if line and not line[0].isspace():
            section = line.split(":")[0].strip()
            continue
        m = re.match(r"^\s+path:\s*['\"]?([^'\"#\s]+)", line)
        if m and section in ("dependencies", "dependency_overrides"):
            dep = os.path.normpath(os.path.join(os.path.dirname(pubspec), m.group(1)))
            if os.path.isfile(os.path.join(dep, "pubspec.yaml")):
                out.append(dep)
    return out


def mobile_roots() -> list[str]:
    """mobile/apps/photos plus its transitive path dependencies."""
    start = os.path.join(REPO, "mobile", "apps", "photos")
    seen: list[str] = []
    todo = [start]
    while todo:
        d = todo.pop()
        if d in seen:
            continue
        seen.append(d)
        todo.extend(_pubspec_path_deps(os.path.join(d, "pubspec.yaml")))
    # Nested packages (e.g. apps/photos/plugins/x) are walked with their
    # parent; drop them so no file is reported twice.
    seen.sort()
    roots: list[str] = []
    for d in seen:
        if not any(d.startswith(r + os.sep) for r in roots):
            roots.append(d)
    return roots


# Rust crates compiled into the shipped apps: the Flutter bridge of the
# mobile app (mobile/apps/photos/flutter_rust_bridge.yaml), the wasm packages
# the web apps import (ente-photos-wasm, ente-cast-wasm, ente-prelogin-wasm)
# and the desktop app's napi addon, plus their path dependencies.
RUST_BINDINGS = ["bindings/frb/photos", "bindings/wasm/photos",
                 "bindings/wasm/cast", "bindings/wasm/prelogin",
                 "bindings/napi/photos"]


def _cargo_path_deps(crate: str, workspace: dict[str, str]) -> list[str]:
    """Path dependencies (normal, build and target-specific; not dev) of a crate."""
    try:
        with open(os.path.join(crate, "Cargo.toml"), "rb") as f:
            manifest = tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError):
        return []
    tables = [manifest.get("dependencies", {}), manifest.get("build-dependencies", {})]
    for t in manifest.get("target", {}).values():
        tables += [t.get("dependencies", {}), t.get("build-dependencies", {})]
    out = []
    for table in tables:
        for name, spec in table.items():
            if not isinstance(spec, dict):
                continue
            if spec.get("workspace") and name in workspace:
                out.append(workspace[name])
            elif "path" in spec:
                out.append(os.path.normpath(os.path.join(crate, spec["path"])))
    return out


def rust_roots() -> list[str]:
    """src/ of the Rust binding crates the apps build, and their path deps."""
    rust = os.path.join(REPO, "rust")
    try:
        with open(os.path.join(rust, "Cargo.toml"), "rb") as f:
            ws_deps = tomllib.load(f).get("workspace", {}).get("dependencies", {})
    except (OSError, tomllib.TOMLDecodeError):
        return []
    workspace = {n: os.path.normpath(os.path.join(rust, d["path"]))
                 for n, d in ws_deps.items() if isinstance(d, dict) and "path" in d}
    seen: list[str] = []
    todo = [os.path.join(rust, b) for b in RUST_BINDINGS]
    while todo:
        d = todo.pop()
        if d in seen:
            continue
        seen.append(d)
        todo.extend(_cargo_path_deps(d, workspace))
    return [os.path.join(d, "src") for d in sorted(seen)]


def source_roots() -> list[str]:
    roots = mobile_roots()
    roots += rust_roots()
    roots += [os.path.join(REPO, "web", "apps", a) for a in WEB_APPS]
    roots.append(os.path.join(REPO, "web", "packages"))
    roots += [os.path.join(REPO, "desktop", p) for p in
              ("src", "build", "package.json", "electron-builder.yml")]
    return [r for r in roots if os.path.exists(r)]


# --------------------------------------------------------------------------
# Lexing: per line, the code without comments and the user-visible parts.


@dataclass
class Line:
    no: int
    code: str  # text outside comments
    visible: str  # string literals / markup text / translation values
    key: str | None = None  # translation key, for translation values


def lex_clike(text: str, ext: str) -> list[Line]:
    """Split C-like source into code (no comments) and string literals.

    A small state machine: // and /* */ comments, '...' "..." (single line),
    `...` (JS templates), ''' / \"\"\" (Dart, Kotlin, Swift). Good enough for
    a grep; a mis-lexed line errs towards scanning more, not less.
    """
    code: list[list[str]] = [[]]
    vis: list[list[str]] = [[]]
    i, n = 0, len(text)
    state = "code"  # code | line | block | str
    quote = ""
    triple_ok = ext in {".dart", ".kt", ".kts", ".swift"}
    while i < n:
        c = text[i]
        if c == "\n":
            code.append([])
            vis.append([])
            if state == "line":
                state = "code"
            elif state == "str" and quote in ("'", '"'):
                state = "code"  # unterminated single-line string
            i += 1
            continue
        if state == "code":
            if text.startswith("//", i) and ext != ".css":
                state = "line"
                i += 2
                continue
            if text.startswith("/*", i):
                state = "block"
                i += 2
                continue
            if triple_ok and (text.startswith("'''", i) or text.startswith('"""', i)):
                state, quote = "str", text[i:i + 3]
                code[-1].append(quote)
                i += 3
                continue
            if c in ("'", '"') or (c == "`" and ext in {".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"}):
                if c == "'" and ext in {".rs"} and re.match(r"'[A-Za-z_]\w*[^'\w]", text[i:i + 40]):
                    code[-1].append(c)  # Rust lifetime
                    i += 1
                    continue
                state, quote = "str", c
                code[-1].append(c)
                i += 1
                continue
            code[-1].append(c)
            i += 1
            continue
        if state == "line":
            i += 1
            continue
        if state == "block":
            if text.startswith("*/", i):
                state = "code"
                i += 2
            else:
                i += 1
            continue
        # string
        if c == "\\" and i + 1 < n and text[i + 1] != "\n":
            code[-1].append(text[i:i + 2])
            vis[-1].append(text[i:i + 2])
            i += 2
            continue
        if text.startswith(quote, i):
            code[-1].append(quote)
            vis[-1].append(" ")
            state = "code"
            i += len(quote)
            continue
        code[-1].append(c)
        vis[-1].append(c)
        i += 1
    if ext in {".tsx", ".jsx"}:
        # JSX text (<p>Some text</p>) is user-visible but not a string
        # literal: add text between tags and lines that are only prose.
        out = []
        for k in range(len(code)):
            c = "".join(code[k])
            extra = _JSX_TEXT.findall(c)
            if _JSX_PROSE_LINE.match(c):
                extra.append(c)
            out.append(Line(k + 1, c, " ".join(["".join(vis[k])] + extra)))
        return out
    return [Line(k + 1, "".join(code[k]), "".join(vis[k])) for k in range(len(code))]


_JSX_TEXT = re.compile(r">([^<>{}]+)<")
_JSX_PROSE_LINE = re.compile(r"^(?!.*\w\.\w)\s*[A-Za-z][^<>{}()=;:\"'`]*\s[^<>{}()=;:\"'`]*[^,;:\s]\s*$")


def lex_markup(text: str) -> list[Line]:
    text = re.sub(r"<!--.*?-->", lambda m: "\n" * m.group(0).count("\n"), text, flags=re.S)
    return [Line(k + 1, s, s) for k, s in enumerate(text.split("\n"))]


def lex_hash(text: str) -> list[Line]:
    out = []
    for k, s in enumerate(text.split("\n")):
        s2 = re.sub(r"(^|\s)#.*$", "", s)
        out.append(Line(k + 1, s2, s2))
    return out


def lex_plain(text: str) -> list[Line]:
    return [Line(k + 1, s, s) for k, s in enumerate(text.split("\n"))]


def _flatten(obj, prefix=""):
    if isinstance(obj, dict):
        for k, v in obj.items():
            yield from _flatten(v, f"{prefix}{k}." if not isinstance(v, str) else f"{prefix}{k}")
    elif isinstance(obj, str):
        yield prefix, obj


def lex_translations(text: str, is_arb: bool) -> list[Line]:
    """Translation values only (not keys, not ARB @metadata), with their key."""
    try:
        data = json.loads(text)
    except ValueError:
        return lex_plain(text)
    lines = text.split("\n")
    vis = [""] * len(lines)
    keys: dict[int, str] = {}
    items = [(k, v) for k, v in data.items() if isinstance(v, str) and not k.startswith("@")] if is_arb \
        else list(_flatten(data))
    cursor = 0
    for key, value in items:
        leaf = key.rsplit(".", 1)[-1]
        pat = re.compile(r'^\s*' + re.escape(json.dumps(leaf)) + r'\s*:')
        for j in list(range(cursor, len(lines))) + list(range(0, cursor)):
            if pat.match(lines[j]):
                vis[j] = value
                keys[j + 1] = key
                cursor = j + 1
                break
    return [Line(k + 1, vis[k], vis[k], keys.get(k + 1)) for k in range(len(lines))]


def is_translation(rel: str) -> bool:
    return rel.endswith(".arb") or (rel.endswith("translation.json") and "/locales/" in rel)


# --------------------------------------------------------------------------
# Allowlist


@dataclass
class Rule:
    paths: list[str]
    terms: list[str]
    category: str
    reason: str
    line: re.Pattern[str] | None = None
    when: str | None = None
    used: int = 0
    index: int = 0


@dataclass
class Exclude:
    path: str
    reason: str
    used: int = 0


@dataclass
class Allowlist:
    categories: dict[str, str]
    rules: list[Rule]
    excludes: list[Exclude]


def load_allowlist(path: str) -> Allowlist:
    with open(path, "rb") as f:
        data = tomllib.load(f)
    cats = data.get("categories", {})
    rules = []
    for idx, e in enumerate(data.get("allow", [])):
        paths = e["path"] if isinstance(e["path"], list) else [e["path"]]
        terms = e["term"] if isinstance(e["term"], list) else [e["term"]]
        for t in terms:
            if t != "*" and t not in TERM_IDS:
                raise SystemExit(f"allowlist entry {idx}: unknown term {t!r}")
        if e["category"] not in cats:
            raise SystemExit(f"allowlist entry {idx}: unknown category {e['category']!r}")
        if not e.get("reason"):
            raise SystemExit(f"allowlist entry {idx}: missing reason")
        rules.append(Rule(paths, terms, e["category"], e["reason"],
                          re.compile(e["line"]) if e.get("line") else None,
                          e.get("when"), 0, idx))
    excludes = [Exclude(e["path"], e["reason"]) for e in data.get("exclude", [])]
    return Allowlist(cats, rules, excludes)


def glob_match(path: str, pattern: str) -> bool:
    # fnmatch's "*" also matches "/", so "a/*" covers the whole subtree.
    return fnmatch.fnmatchcase(path, pattern)


# --------------------------------------------------------------------------
# Scanning


@dataclass
class Hit:
    term: str
    path: str  # repo-relative, "apk:<member>" or "web-out:<dir>/<file>"
    line: int
    text: str  # the line (or extracted string), trimmed
    key: str | None = None  # translation key, if any
    key_referenced: bool | None = None
    rule: Rule | None = None


def scan_lines(rel: str, lines: list[Line], hits: list[Hit], only: set[str] | None = None,
               terms: list[tuple[str, re.Pattern[str], str]] = TERMS) -> None:
    for ln in lines:
        for term, rx, ctx in terms:
            if only is not None and term not in only:
                continue
            hay = ln.code if ctx == "code" else ln.visible
            if hay and rx.search(hay):
                snippet = hay.strip()
                hits.append(Hit(term, rel, ln.no, snippet[:200], ln.key))


_RS_CFG_TEST = re.compile(r"^\s*#\[cfg\(test\)\]\s*$")
_RS_MOD_OPEN = re.compile(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?mod\s+\w+\s*\{")
_RS_STR = re.compile(r'"(?:\\.|[^"\\])*"')


def drop_rust_test_modules(lines: list[Line]) -> list[Line]:
    """Drop `#[cfg(test)] mod x { ... }` blocks (inline unit tests, not shipped)."""
    out: list[Line] = []
    k = 0
    while k < len(lines):
        if _RS_CFG_TEST.match(lines[k].code):
            j = k + 1
            while j < len(lines) and not lines[j].code.strip():
                j += 1
            if j < len(lines) and _RS_MOD_OPEN.match(lines[j].code):
                depth = 0
                while j < len(lines):
                    c = _RS_STR.sub("", lines[j].code)
                    depth += c.count("{") - c.count("}")
                    j += 1
                    if depth <= 0:
                        break
                k = j
                continue
        out.append(lines[k])
        k += 1
    return out


def lex_file(rel: str, text: str) -> list[Line]:
    ext = os.path.splitext(rel)[1].lower()
    if is_translation(rel):
        return lex_translations(text, rel.endswith(".arb"))
    if ext == ".rs":
        return drop_rust_test_modules(lex_clike(text, ext))
    if ext in CLIKE:
        return lex_clike(text, ext)
    if ext in MARKUP:
        return lex_markup(text)
    if ext in HASHCOMMENT or os.path.basename(rel) in ("Podfile", "_headers", "_redirects"):
        return lex_hash(text)
    return lex_plain(text)


def excluded(rel: str, allow: Allowlist) -> bool:
    for ex in allow.excludes:
        if glob_match(rel, ex.path):
            ex.used += 1
            return True
    return False


def iter_source_files(allow: Allowlist):
    for root in source_roots():
        if os.path.isfile(root):
            yield root
            continue
        for d, dirs, files in os.walk(root):
            reld = os.path.relpath(d, REPO).replace(os.sep, "/")
            # Prune excluded directories early (node_modules, build, ...).
            dirs[:] = sorted(x for x in dirs if not excluded(f"{reld}/{x}/", allow))
            for fn in sorted(files):
                ext = os.path.splitext(fn)[1].lower()
                if ext in SCANNED_EXT or fn in SCANNED_NAMES:
                    yield os.path.join(d, fn)


def scan_source(allow: Allowlist, hits: list[Hit]) -> int:
    count = 0
    for p in iter_source_files(allow):
        rel = os.path.relpath(p, REPO).replace(os.sep, "/")
        if excluded(rel, allow):
            continue
        try:
            with open(p, encoding="utf-8") as f:
                text = f.read()
        except (UnicodeDecodeError, OSError):
            continue
        count += 1
        scan_lines(rel, lex_file(rel, text), hits)
    return count


# Translation keys: is the key referenced by the code that is scanned?
_ident_cache: set[str] | None = None


def referenced_identifiers(allow: Allowlist) -> set[str]:
    global _ident_cache
    if _ident_cache is None:
        ids: set[str] = set()
        for p in iter_source_files(allow):
            if not p.endswith((".dart", ".ts", ".tsx", ".js", ".mjs")):
                continue
            rel = os.path.relpath(p, REPO).replace(os.sep, "/")
            # Generated l10n accessors define every key; they do not count
            # as a use.
            if "/generated/" in rel or "/l10n/" in rel or rel.endswith(".g.dart"):
                continue
            try:
                with open(p, encoding="utf-8") as f:
                    text = f.read()
            except (UnicodeDecodeError, OSError):
                continue
            ids.update(re.findall(r"[A-Za-z_][\w.]*", text))
            ids.update(re.findall(r"""["'`]([\w.]+)["'`]""", text))
        _ident_cache = ids
    return _ident_cache


def key_is_referenced(key: str, allow: Allowlist) -> bool:
    ids = referenced_identifiers(allow)
    # Dart: context.l10n.key / S.of(context).key; web: t("ns.key") or "key".
    leaf = key.rsplit(".", 1)[-1]
    if key in ids:
        return True
    return any(i.endswith("." + leaf) or i == leaf for i in ids)


# Binary string extraction (APK).
_ASCII_RUN = re.compile(rb"[\x20-\x7e]{4,}")
_U16_RUN = re.compile(rb"(?:[\x20-\x7e]\x00){4,}")


def binary_strings(data: bytes) -> list[str]:
    out = [m.group(0).decode("ascii") for m in _ASCII_RUN.finditer(data)]
    out += [m.group(0).decode("utf-16-le") for m in _U16_RUN.finditer(data)]
    return out


_TOKEN_CHAR = re.compile(r"[\w.\-/:@%?=&+~#]")


def binary_lines(data: bytes, only: set[str] | None) -> list[Line]:
    """Printable strings of a binary, one "line" each (numbered by index).

    Native code keeps string literals back to back without separators
    (Rust), so a long run is cut down to the token around each match (a
    URL, a host, an e-mail address), up to 120 characters.
    """
    terms = [rx for t, rx, _ in BINARY_TERM_RX if only is None or t in only]
    out: list[Line] = []
    for k, s in enumerate(binary_strings(data)):
        if len(s) <= 200:
            out.append(Line(k + 1, s, s))
            continue
        for rx in terms:
            for m in rx.finditer(s):
                a, b = m.span()
                while a > 0 and m.start() - a < 60 and _TOKEN_CHAR.match(s[a - 1]):
                    a -= 1
                # Start at the URL scheme when there is one: the run may hold
                # other URLs right before this one.
                scheme = s.rfind("http", a, m.start())
                if scheme >= 0:
                    a = scheme
                while b < len(s) and b - m.end() < 60 and _TOKEN_CHAR.match(s[b]):
                    b += 1
                nxt = s.find("http", m.end(), b)
                if nxt >= 0:
                    b = nxt
                out.append(Line(k + 1, s[a:b], s[a:b]))
    return out


# Terms checked inside compiled code and binary resources. The "visible"
# terms are not: compiled code is full of identifiers such as `subscription`
# that are not text, and the visible text itself is checked in the sources
# (translations, string literals) and in the text assets below.
BINARY_TERMS = {t for t, _, ctx in TERMS if ctx == "code"} | {"support@ente"}


def scan_apk(apk: str, allow: Allowlist, hits: list[Hit]) -> int:
    count = 0
    with zipfile.ZipFile(apk) as z:
        for info in z.infolist():
            name = info.filename
            if info.is_dir():
                continue
            rel = f"apk:{name}"
            if excluded(rel, allow):
                continue
            ext = os.path.splitext(name)[1].lower()
            base = os.path.basename(name)
            # Native code: libapp.so (the Dart AOT snapshot) and Photovault's
            # own Rust libraries. The other .so files are third-party
            # (Flutter engine, ffmpeg, ...) and not scanned.
            own_so = ext == ".so" and (base == "libapp.so" or re.match(r"lib(ente|rust)", base))
            binary = (own_so or name.endswith((".dex", ".arsc")) or base == "AndroidManifest.xml"
                      or (name.startswith("res/") and ext == ".xml"))
            text_asset = name.startswith("assets/") and (ext in SCANNED_EXT or ext in {".md", ".txt"})
            if not (binary or text_asset):
                continue
            data = z.read(info)
            count += 1
            if binary:
                only = BINARY_TERMS
                if name.endswith(".arsc") or base == "AndroidManifest.xml" or name.startswith("res/"):
                    only = None  # Android resources are user-visible text.
                scan_lines(rel, binary_lines(data, only), hits, only=only, terms=BINARY_TERM_RX)
            else:
                try:
                    text = data.decode("utf-8")
                except UnicodeDecodeError:
                    continue
                scan_lines(rel, lex_file(name, text), hits)
    return count


_JS_JSON_PARSE = re.compile(r"JSON\.parse\('((?:[^'\\\n]|\\.)*)'\)")
_JS_ESCAPE = re.compile(r"\\(x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f]+\}|u[0-9A-Fa-f]{4}|.)", re.S)


def _js_unescape(body: str) -> str:
    def rep(m: re.Match[str]) -> str:
        e = m.group(1)
        if e[0] == "x":
            return chr(int(e[1:], 16))
        if e.startswith("u{"):
            return chr(int(e[2:-1], 16))
        if e[0] == "u":
            return chr(int(e[1:], 16))
        return {"n": "\n", "t": "\t", "r": "\r", "0": "\0"}.get(e, e)
    return _JS_ESCAPE.sub(rep, body)


def js_bundle_lines(text: str, only: set[str]) -> list[Line]:
    """The parts of a minified JS bundle worth reporting, one per "line".

    Minified lines can be megabytes, and a tokenizer cannot tell a regex
    literal from a division without parsing, so:
      * translation files, which Next.js inlines as JSON.parse('{...}'), are
        decoded and yield one line per translation with its key (so the
        translation-key rules of the allowlist apply to the bundle too);
      * elsewhere, each match of a term yields the string around it (from
        the nearest quote before it to the same quote after it, or 60
        characters either side), reported at the file's line number.
    """
    starts = [0] + [m.end() for m in re.finditer("\n", text)]

    def lineno(off: int) -> int:
        lo, hi = 0, len(starts)
        while lo + 1 < hi:
            mid = (lo + hi) // 2
            if starts[mid] <= off:
                lo = mid
            else:
                hi = mid
        return lo + 1

    out: list[Line] = []
    skip: list[tuple[int, int]] = []
    for m in _JS_JSON_PARSE.finditer(text):
        try:
            data = json.loads(_js_unescape(m.group(1)))
        except ValueError:
            continue
        if not isinstance(data, dict):
            continue
        skip.append(m.span())
        no = lineno(m.start())
        for k, v in _flatten(data):
            out.append(Line(no, v, v, k))

    combined = re.compile("|".join(f"(?:{rx.pattern})" for t, rx, _ in TERMS if t in only), re.I)
    seen: set[tuple[int, int]] = set()
    for m in combined.finditer(text):
        s, e = m.span()
        if any(a <= s < b for a, b in skip):
            continue
        a = max(text.rfind(q, max(0, s - 300), s) for q in "\"'`")
        b = text.find(text[a], e, e + 300) if a >= 0 else -1
        if a < 0 or b < 0:
            a, b = max(0, s - 61), min(len(text), e + 60)
        if (a, b) in seen:
            continue
        seen.add((a, b))
        window = text[a + 1:b]
        out.append(Line(lineno(s), window, window))
    return out


WEB_OUT_EXT = {".js", ".mjs", ".html", ".json", ".css", ".txt", ".webmanifest", ".xml", ".svg"}


def scan_web_out(dirs: list[str], allow: Allowlist, hits: list[Hit]) -> int:
    count = 0
    for top in dirs:
        top = os.path.abspath(top)
        label = os.path.relpath(top, REPO).replace(os.sep, "/") if top.startswith(REPO + os.sep) \
            else os.path.basename(top)
        for d, dirs_, files in os.walk(top):
            dirs_.sort()
            for fn in sorted(files):
                ext = os.path.splitext(fn)[1].lower()
                if ext not in WEB_OUT_EXT and fn not in SCANNED_NAMES:
                    continue
                p = os.path.join(d, fn)
                rel = "web-out:" + label + "/" + os.path.relpath(p, top).replace(os.sep, "/")
                if excluded(rel, allow):
                    continue
                try:
                    with open(p, encoding="utf-8") as f:
                        text = f.read()
                except (UnicodeDecodeError, OSError):
                    continue
                count += 1
                if ext in (".js", ".mjs", ".css"):
                    # Minified bundles: one line can be megabytes. Split into
                    # string literals so a hit points at something readable.
                    only = BINARY_TERMS | {"ente"}
                    scan_lines(rel, js_bundle_lines(text, only), hits, only=only)
                else:
                    scan_lines(rel, lex_file(fn, text), hits)
    return count


# --------------------------------------------------------------------------
# Matching and report


_translations: list[tuple[str, str]] | None = None


def translation_values(allow: Allowlist) -> list[tuple[str, str]]:
    """(key, value) of every source translation (.arb, translation.json)."""
    global _translations
    if _translations is None:
        _translations = []
        for p in iter_source_files(allow):
            rel = os.path.relpath(p, REPO).replace(os.sep, "/")
            if not is_translation(rel):
                continue
            try:
                with open(p, encoding="utf-8") as f:
                    text = f.read()
            except (UnicodeDecodeError, OSError):
                continue
            _translations += [(ln.key, ln.visible) for ln in lex_translations(text, rel.endswith(".arb"))
                              if ln.key]
    return _translations


def attach_translation_key(h: Hit, allow: Allowlist) -> None:
    """For a string found in a build output, the translation it came from.

    Compiled code keeps a translation as one string, or as the pieces
    between its placeholders. If every translation containing the string
    is unreferenced, the hit gets one of their keys (so the
    translation-key rule can explain it); if any is referenced, it gets
    that one (so it cannot).
    """
    text = h.text.strip()
    if len(text) < 8:
        return
    found = [k for k, v in translation_values(allow) if text in v]
    if not found:
        return
    referenced = [k for k in found if key_is_referenced(k, allow)]
    h.key = (referenced or found)[0]
    h.key_referenced = bool(referenced)


def explain(hits: list[Hit], allow: Allowlist) -> None:
    for h in hits:
        if h.key is None and h.path.startswith(("apk:", "web-out:")):
            attach_translation_key(h, allow)
        for r in allow.rules:
            if h.term not in r.terms and "*" not in r.terms:
                continue
            if not any(glob_match(h.path, p) for p in r.paths):
                continue
            if r.line and not r.line.search(h.text):
                continue
            if r.when == "translation-key-unreferenced":
                if h.key is None:
                    continue
                if h.key_referenced is None:
                    h.key_referenced = key_is_referenced(h.key, allow)
                if h.key_referenced:
                    continue
            elif r.when is not None:
                raise SystemExit(f"allowlist entry {r.index}: unknown when={r.when!r}")
            h.rule = r
            r.used += 1
            break


def md_escape(s: str) -> str:
    return s.replace("\\", "\\\\").replace("|", "\\|").replace("<", "&lt;").replace("`", "'")


def render(hits: list[Hit], allow: Allowlist, scanned: dict[str, int], args) -> str:
    explained = [h for h in hits if h.rule]
    unexplained = [h for h in hits if not h.rule]
    out: list[str] = []
    w = out.append
    w("# Strip report")
    w("")
    w("<!-- Generated by scripts/strip-report.py; do not edit by hand. -->")
    w("")
    w("Every place where the Photovault apps still mention Ente, an Ente host, or a "
      "stripped company service (billing, referrals, crash reporting, store links), "
      "and why it stays. Generated by `python3 -I scripts/strip-report.py` from the "
      "explanations in `scripts/strip-allowlist.toml`; CI runs it with `--check`, "
      "which fails on any unexplained hit.")
    w("")
    w("Scanned: " + ", ".join(f"{k}: {v} files" for k, v in scanned.items()) + ".")
    w("Comments are not scanned. Terms marked *visible* only count in text a user "
      "can see (string literals, markup, translation values); the others count "
      "anywhere outside comments.")
    w("")
    w("## Summary")
    w("")
    w(f"{len(hits)} hits, {len(explained)} explained, **{len(unexplained)} unexplained**.")
    w("")
    w("| Term | Context | Hits | Unexplained |")
    w("| --- | --- | ---: | ---: |")
    for term, _, ctx in TERMS:
        n = sum(1 for h in hits if h.term == term)
        u = sum(1 for h in unexplained if h.term == term)
        w(f"| {md_escape(term)} | {ctx} | {n} | {u} |")
    w("")
    w("| Category | Meaning | Hits |")
    w("| --- | --- | ---: |")
    for cat, meaning in allow.categories.items():
        n = sum(1 for h in explained if h.rule.category == cat)
        w(f"| {cat} | {md_escape(meaning)} | {n} |")
    w("")
    w("## Unexplained")
    w("")
    if unexplained:
        w("| Term | Where | Text |")
        w("| --- | --- | --- |")
        for h in unexplained:
            w(f"| {md_escape(h.term)} | `{md_escape(h.path)}:{h.line}` | {md_escape(h.text[:120])} |")
    else:
        w("None.")
    w("")
    w("## Explained hits")
    w("")
    w("| Term | Where | Category | Explanation |")
    w("| --- | --- | --- | --- |")
    for h in sorted(explained, key=lambda h: (h.rule.category, h.path, h.line, h.term)):
        w(f"| {md_escape(h.term)} | `{md_escape(h.path)}:{h.line}` | {h.rule.category} | {md_escape(h.rule.reason)} |")
    w("")
    w("## Excluded paths")
    w("")
    w("Not scanned at all (build output, tests, generated code):")
    w("")
    w("| Path | Reason |")
    w("| --- | --- |")
    for ex in allow.excludes:
        w(f"| `{md_escape(ex.path)}` | {md_escape(ex.reason)} |")
    unused = [r for r in allow.rules if r.used == 0 and _rule_applies(r, args)]
    if unused:
        w("")
        w("## Allowlist entries that matched nothing")
        w("")
        w("Stale entries; delete them from `scripts/strip-allowlist.toml`.")
        w("")
        for r in unused:
            w(f"- entry {r.index}: `{md_escape(', '.join(r.paths))}` ({', '.join(r.terms)})")
    w("")
    return "\n".join(out)


def _rule_applies(r: Rule, args) -> bool:
    """Was the input this rule is about scanned in this run?"""
    for p in r.paths:
        if p.startswith("apk:"):
            if args.apk:
                return True
        elif p.startswith("web-out:"):
            if args.web_out:
                return True
        elif not args.no_source:
            return True
    return False


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0] if __doc__ else None)
    ap.add_argument("--check", action="store_true", help="exit 1 if any hit is unexplained")
    ap.add_argument("--apk", help="also scan this built APK")
    ap.add_argument("--web-out", nargs="+", default=[], metavar="DIR",
                    help="also scan these web static-export directories")
    ap.add_argument("--no-source", action="store_true", help="skip the source tree")
    ap.add_argument("--allowlist", default=DEFAULT_ALLOWLIST)
    ap.add_argument("--output", default=DEFAULT_OUTPUT,
                    help="report path (default docs/src/strip-report.md; '-' for stdout)")
    args = ap.parse_args()

    allow = load_allowlist(args.allowlist)
    hits: list[Hit] = []
    scanned: dict[str, int] = {}
    if not args.no_source:
        scanned["source"] = scan_source(allow, hits)
    if args.apk:
        scanned["apk"] = scan_apk(args.apk, allow, hits)
    if args.web_out:
        scanned["web export"] = scan_web_out(args.web_out, allow, hits)
    explain(hits, allow)
    report = render(hits, allow, scanned, args)
    if args.output == "-":
        sys.stdout.write(report)
    else:
        os.makedirs(os.path.dirname(os.path.abspath(args.output)), exist_ok=True)
        with open(args.output, "w", encoding="utf-8") as f:
            f.write(report)
    unexplained = [h for h in hits if not h.rule]
    print(f"strip-report: {len(hits)} hits, {len(unexplained)} unexplained "
          f"({', '.join(f'{k} {v} files' for k, v in scanned.items())}); report: {args.output}",
          file=sys.stderr)
    for h in unexplained[:2000]:
        print(f"  UNEXPLAINED {h.term}: {h.path}:{h.line}: {h.text[:160]!r}", file=sys.stderr)
    if len(unexplained) > 2000:
        print(f"  ... and {len(unexplained) - 2000} more", file=sys.stderr)
    return 1 if (args.check and unexplained) else 0


if __name__ == "__main__":
    sys.exit(main())
