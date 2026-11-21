#!/usr/bin/env python3
"""Comment ratchet: no new comments, and the count only goes down.

usage: comment_ratchet.py [--root DIR] (--check | --update | --list [PATH ...])

  --check   fail if any file has more comment lines than lint/comment-baseline.txt
            allows, or fewer (lower the baseline with --update so it stays down)
  --update  rewrite the baseline from the current tree
  --list    print every counted comment line, optionally only under PATHs

Counted: Rust //, ///, //!, /* */; # in Python, shell, Nix, TOML, YAML and
systemd units; Python docstrings. Not counted, because they are output rather
than comments: doc comments on Rust types deriving JsonSchema (schema/v1
descriptions) or clap's Parser/Subcommand/Args/ValueEnum (--help text), a
Python module docstring the script prints through __doc__, shebangs, and
tool directives (# noqa, # type:, # shellcheck).
Shipped migrations are skipped: sqlx checksums them, so they cannot change.
"""

import os
import re
import sys
from pathlib import Path

BASELINE = "lint/comment-baseline.txt"
HASH_LANGS = {".py", ".sh", ".nix", ".toml", ".yml", ".yaml", ".service", ".timer"}
SKIP_DIRS = {".git", "target", "result", ".direnv", ".loker-dev", "node_modules", "__pycache__"}
SKIP_FILES = {"Cargo.lock", "flake.lock"}
SKIP_PREFIXES = ("crates/loker-core/migrations/",)
DIRECTIVE_RE = re.compile(r"#\s*(noqa|type:|pragma|pyright:|shellcheck )")
OUTPUT_DERIVES = {"JsonSchema", "Parser", "Subcommand", "Args", "ValueEnum"}


def tracked_files(root: Path):
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS and not d.startswith(".loker"))
        for name in sorted(filenames):
            path = Path(dirpath, name)
            rel = path.relative_to(root).as_posix()
            if name in SKIP_FILES or rel.startswith(SKIP_PREFIXES):
                continue
            if path.suffix == ".rs" or path.suffix in HASH_LANGS:
                yield rel, path


def rust_comment_lines(text: str):
    i, n, line = 0, len(text), 1
    flagged = {}

    def flag(ln, doc):
        flagged[ln] = flagged.get(ln, False) or doc

    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
        elif text.startswith("//", i):
            doc = text.startswith("///", i) and not text.startswith("////", i) or text.startswith("//!", i)
            flag(line, doc)
            while i < n and text[i] != "\n":
                i += 1
        elif text.startswith("/*", i):
            depth, i = 1, i + 2
            flag(line, False)
            while i < n and depth:
                if text[i] == "\n":
                    line += 1
                    flag(line, False)
                if text.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif text.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
        elif m := re.compile(r'b?r(#*)"').match(text, i):
            if i > 0 and (text[i - 1].isalnum() or text[i - 1] == "_"):
                i += 1
                continue
            end = '"' + m.group(1)
            j = text.find(end, m.end())
            j = n if j < 0 else j + len(end)
            line += text.count("\n", i, j)
            i = j
        elif c == '"':
            i += 1
            while i < n and text[i] != '"':
                if text[i] == "\\":
                    i += 1
                if i < n and text[i] == "\n":
                    line += 1
                i += 1
            i += 1
        elif c == "'":
            if text.startswith("\\", i + 1):
                j = text.find("'", i + 3)
                i = j + 1 if j > 0 else i + 1
            elif i + 2 < n and text[i + 2] == "'":
                i += 3
            else:
                i += 1
        else:
            i += 1
    return sorted(flagged.items())


DERIVE_RE = re.compile(r"#\[derive\(([^\]]*)\)\]", re.S)
TYPE_RE = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?(struct|enum)\b")


def rust_output_doc_lines(text: str):
    lines = text.split("\n")
    out = set()
    open_bodies = []
    depth, attrs, docs = 0, [], []
    i = 0
    while i < len(lines):
        raw = lines[i]
        s = raw.strip()
        i += 1
        if s.startswith("///"):
            docs.append(i)
            continue
        if s.startswith("#["):
            attr = s
            while attr.count("[") > attr.count("]") and i < len(lines):
                attr += lines[i].strip()
                i += 1
            attrs.append(attr)
            continue
        if not s or s.startswith("//"):
            continue
        derives = {d.strip().split("::")[-1] for m in DERIVE_RE.finditer(" ".join(attrs)) for d in m.group(1).split(",")}
        inside_output = bool(open_bodies) and open_bodies[-1][0] == depth and open_bodies[-1][1]
        if TYPE_RE.match(raw):
            is_output = bool(derives & OUTPUT_DERIVES)
            if is_output:
                out.update(docs)
            if "{" in raw:
                open_bodies.append((depth + 1, is_output))
        elif inside_output:
            out.update(docs)
            if "{" in raw:
                open_bodies.append((depth + 1, True))
        docs, attrs = [], []
        depth += raw.count("{") - raw.count("}")
        while open_bodies and depth < open_bodies[-1][0]:
            open_bodies.pop()
    return out


def hash_comment_lines(text: str, suffix: str):
    lines = text.split("\n")
    counted = []
    in_triple = None
    for idx, raw in enumerate(lines, start=1):
        if idx == 1 and raw.startswith("#!"):
            continue
        quote, k = None, 0
        if in_triple:
            end = raw.find(in_triple)
            if end < 0:
                continue
            k, in_triple = end + 3, None
        while k < len(raw):
            ch = raw[k]
            if quote:
                if ch == "\\":
                    k += 2
                    continue
                if ch == quote:
                    quote = None
            elif suffix == ".py" and raw.startswith(('"""', "'''"), k):
                end = raw.find(raw[k : k + 3], k + 3)
                if end < 0:
                    in_triple = raw[k : k + 3]
                    break
                k = end + 3
                continue
            elif ch in "\"'" and not (suffix == ".nix" and raw.startswith("''", k)):
                quote = ch
            elif ch == "#" and (k == 0 or raw[k - 1].isspace()):
                if not DIRECTIVE_RE.match(raw, k):
                    counted.append(idx)
                break
            k += 1
    return counted


def python_docstring_lines(text: str):
    lines = text.split("\n")
    out = []
    module_doc_is_output = "__doc__" in text
    prev = ""
    first_stmt = True
    i = 0
    while i < len(lines):
        s = lines[i].strip()
        opener = re.match(r'^[rRbBuU]?("""|\'\'\')', s)
        starts_body = prev.endswith(":") and re.match(r"^(async\s+def|def|class)\b", prev_stmt_head(lines, i))
        if opener and (first_stmt or starts_body):
            q = opener.group(1)
            j = i
            rest = s[opener.end():]
            if q not in rest:
                j = i + 1
                while j < len(lines) and q not in lines[j]:
                    j += 1
            if not (first_stmt and module_doc_is_output):
                out.extend(range(i + 1, j + 2))
            i = j + 1
            first_stmt = False
            prev = lines[j].strip() if j < len(lines) else ""
            continue
        if s and not s.startswith("#"):
            first_stmt = False
            prev = s
        i += 1
    return out


def prev_stmt_head(lines, i):
    j = i - 1
    while j >= 0 and not lines[j].strip():
        j -= 1
    depth = 0
    while j >= 0:
        s = lines[j]
        depth += s.count(")") - s.count("(")
        if depth <= 0:
            return s.strip()
        j -= 1
    return ""


def comment_lines(rel: str, text: str):
    suffix = Path(rel).suffix
    if suffix == ".rs":
        output = rust_output_doc_lines(text)
        return [ln for ln, doc in rust_comment_lines(text) if not (doc and ln in output)]
    lines = set(hash_comment_lines(text, suffix))
    if suffix == ".py":
        lines.update(python_docstring_lines(text))
    return sorted(lines)


def scan(root: Path):
    counts = {}
    for rel, path in tracked_files(root):
        found = comment_lines(rel, path.read_text(encoding="utf-8", errors="replace"))
        if found:
            counts[rel] = found
    return counts


def read_baseline(root: Path):
    allowed = {}
    path = root / BASELINE
    if path.exists():
        for line in path.read_text().splitlines():
            if line.strip() and not line.startswith("#"):
                count, rel = line.split(None, 1)
                allowed[rel] = int(count)
    return allowed


def main(argv):
    root = Path.cwd()
    if "--root" in argv:
        k = argv.index("--root")
        root = Path(argv[k + 1])
        argv = argv[:k] + argv[k + 2 :]
    if not argv or argv[0] not in {"--check", "--update", "--list"}:
        sys.exit(__doc__)
    counts = scan(root)
    mode = argv[0]

    if mode == "--list":
        prefixes = tuple(argv[1:])
        for rel, found in sorted(counts.items()):
            if prefixes and not rel.startswith(prefixes):
                continue
            text = (root / rel).read_text(encoding="utf-8", errors="replace").split("\n")
            for ln in found:
                print(f"{rel}:{ln}: {text[ln - 1].strip()}")
        return 0

    if mode == "--update":
        body = "".join(f"{len(found)} {rel}\n" for rel, found in sorted(counts.items()))
        (root / BASELINE).write_text(body)
        total = sum(len(f) for f in counts.values())
        print(f"baseline: {total} comment lines in {len(counts)} files")
        return 0

    allowed = read_baseline(root)
    errors = []
    for rel in sorted(set(counts) | set(allowed)):
        have, may = len(counts.get(rel, [])), allowed.get(rel, 0)
        if have > may:
            errors.append(f"{rel}: {have} comment lines, baseline allows {may}. Say it in code, a test, or the docs.")
        elif have < may:
            errors.append(f"{rel}: {have} comment lines, baseline still says {may}. Run --update to lock in the drop.")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        print(f"\ncomment ratchet failed ({BASELINE}). See .claude/skills/delete-comment.", file=sys.stderr)
        return 1
    print(f"comment ratchet ok: {sum(allowed.values())} comment lines allowed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
