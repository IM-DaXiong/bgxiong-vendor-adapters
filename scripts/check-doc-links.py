#!/usr/bin/env python3
"""Check intra-repository links in tracked and untracked Markdown.

Fails on real navigation defects outside fenced code:

- relative Markdown links and HTML src/href whose target is not in the tree
- anchors that do not match a heading in that target
- inline code spans that are repository paths and do not exist

Does not fail on:

- fenced code (commands and sample paths); pass --show-examples to list them
- http(s) and mailto URLs (not fetched)
- inline spans that contain spaces, globs, ellipsis, or placeholders
- untracked build outputs (dist-pack, target/, *.zip)
- API routes and JSON pointers that start with `/`
- names that are not a repo-relative path and not a file path with a known extension

Usage:
  python scripts/check-doc-links.py
  python scripts/check-doc-links.py --show-examples
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOP_LEVEL = {
    "AGENT.md",
    "AGENTS.md",
    "LICENSE",
    "README.md",
    "community",
    "contract",
    "docs",
    "examples",
    "packages",
    "projects",
    "scripts",
}
GENERATED_PARTS = {"dist-pack", "dist-pack-b", "dist-pack-d", "dist-offline-h1", "target"}
DOC_EXTENSIONS = (
    ".md",
    ".json",
    ".wit",
    ".rs",
    ".py",
    ".toml",
    ".ps1",
    ".txt",
    ".yml",
    ".yaml",
)
FENCE_RE = re.compile(r"^(```+|~~~+)")
LINK_RE = re.compile(r"(!?)\[([^\]]*)\]\(([^)]+)\)")
HTML_REF_RE = re.compile(
    r"""<(?:img|a)\b[^>]*\b(?:src|href)\s*=\s*["']([^"']+)["']""",
    re.IGNORECASE,
)
CODE_RE = re.compile(r"`([^`\n]+)`")
HEADING_RE = re.compile(r"^(#{1,6})\s+(.+?)\s*#*\s*$")
SKIP_CODE_CHARS = set("*<>$=|{}")


def git_markdown() -> list[str]:
    out = subprocess.check_output(
        ["git", "ls-files", "-co", "--exclude-standard", "*.md"],
        cwd=ROOT,
        text=True,
    )
    return [line for line in out.splitlines() if line]


def github_slug(text: str) -> str:
    text = re.sub(r"<[^>]+>", "", text)
    text = text.replace("`", "")
    text = text.strip().lower()
    kept: list[str] = []
    for ch in text:
        cat = unicodedata.category(ch)
        if cat.startswith("L") or cat.startswith("N") or ch in " -":
            kept.append(ch)
    slug = "".join(kept).strip()
    # Match GitHub: punctuation is removed first, then each remaining space
    # becomes a hyphen, so "Track + Crop" stays "track--crop".
    slug = slug.replace(" ", "-")
    return slug.strip("-")


def heading_slugs(path: Path) -> set[str]:
    if path.suffix.lower() not in {".md", ".markdown"} or not path.is_file():
        return set()
    counts: dict[str, int] = {}
    slugs: set[str] = set()
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        match = HEADING_RE.match(line)
        if not match:
            continue
        base = github_slug(match.group(2))
        if not base:
            continue
        n = counts.get(base, 0)
        counts[base] = n + 1
        slugs.add(base if n == 0 else f"{base}-{n}")
    return slugs


def split_target(raw: str) -> tuple[str, str]:
    url = raw.strip()
    if url.startswith("<") and url.endswith(">"):
        url = url[1:-1].strip()
    if len(url) >= 2 and url[0] == url[-1] and url[0] in {'"', "'"}:
        url = url[1:-1]
    # Markdown title: (url "title") or (url 'title')
    spaced = re.match(r'^(\S+)\s+[\'"].*[\'"]$', url)
    if spaced:
        url = spaced.group(1)
    path, _, anchor = url.partition("#")
    return path, anchor


def is_external(url: str) -> bool:
    lowered = url.lower()
    return lowered.startswith(("http://", "https://", "mailto:", "ftp://"))


def resolve_path(md_rel: str, path_part: str) -> Path | None:
    if not path_part:
        return ROOT / md_rel
    normalized = path_part.replace("\\", "/")
    relative = (ROOT / md_rel).parent / normalized
    relative = Path(os.path.normpath(relative))
    if relative.exists():
        return relative
    first = normalized.split("/", 1)[0]
    if not normalized.startswith(".") and first in TOP_LEVEL:
        rooted = Path(os.path.normpath(ROOT / normalized))
        if rooted.exists():
            return rooted
    return relative


def is_generated(path_part: str) -> bool:
    parts = set(path_part.replace("\\", "/").split("/"))
    if parts & GENERATED_PARTS:
        return True
    return path_part.endswith(".zip")


def fence_ranges(lines: list[str]) -> list[tuple[int, int]]:
    ranges: list[tuple[int, int]] = []
    start = None
    marker = None
    for idx, line in enumerate(lines, 1):
        stripped = line.strip()
        match = FENCE_RE.match(stripped)
        if not match:
            continue
        token = match.group(1)[0]
        if start is None:
            start = idx
            marker = token
        elif token == marker:
            ranges.append((start, idx))
            start = None
            marker = None
    if start is not None:
        ranges.append((start, len(lines) + 1))
    return ranges


def in_fence(ranges: list[tuple[int, int]], line_no: int) -> bool:
    return any(a <= line_no <= b for a, b in ranges)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--show-examples",
        action="store_true",
        help="print fenced sample paths that do not exist (not a failure)",
    )
    args = parser.parse_args()

    errors: list[str] = []
    examples: list[str] = []
    external = 0
    checked = 0
    slug_cache: dict[Path, set[str]] = {}

    for rel in git_markdown():
        text = (ROOT / rel).read_text(encoding="utf-8", errors="replace")
        lines = text.splitlines()
        ranges = fence_ranges(lines)
        for lineno, line in enumerate(lines, 1):
            fenced = in_fence(ranges, lineno)
            refs: list[tuple[str, str]] = []
            for match in LINK_RE.finditer(line):
                refs.append(("link", match.group(3).strip()))
            for match in HTML_REF_RE.finditer(line):
                refs.append(("html", match.group(1).strip()))
            for kind, raw in refs:
                path_part, anchor = split_target(raw)
                if is_external(path_part or raw):
                    external += 1
                    continue
                if path_part.startswith("//"):
                    external += 1
                    continue
                checked += 1
                target = resolve_path(rel, path_part)
                if target is None or not target.exists():
                    bucket = examples if fenced else errors
                    bucket.append(f"{rel}:{lineno}: missing {kind} {raw}")
                    continue
                if anchor:
                    key = target.resolve()
                    if key not in slug_cache:
                        slug_cache[key] = heading_slugs(target)
                    if anchor not in slug_cache[key]:
                        bucket = examples if fenced else errors
                        bucket.append(
                            f"{rel}:{lineno}: missing anchor #{anchor} in {target.relative_to(ROOT)}"
                        )
            if fenced:
                continue
            for match in CODE_RE.finditer(line):
                span = match.group(1).strip()
                if not span or any(ch.isspace() for ch in span):
                    continue
                if any(ch in span for ch in SKIP_CODE_CHARS) or "..." in span or "…" in span:
                    continue
                if is_external(span) or span.startswith("#") or span.startswith("/"):
                    continue
                if is_generated(span):
                    continue
                normalized = span.replace("\\", "/")
                first = normalized.split("/", 1)[0]
                looks_like_repo = normalized.startswith(("./", "../")) or first in TOP_LEVEL
                looks_like_file = "/" in normalized and normalized.lower().endswith(DOC_EXTENSIONS)
                if not looks_like_repo and not looks_like_file:
                    continue
                path_part, anchor = split_target(span)
                if not path_part:
                    continue
                checked += 1
                target = resolve_path(rel, path_part)
                if target is None or not target.exists():
                    errors.append(f"{rel}:{lineno}: missing path `{span}`")
                    continue
                if anchor:
                    key = target.resolve()
                    if key not in slug_cache:
                        slug_cache[key] = heading_slugs(target)
                    if anchor not in slug_cache[key]:
                        errors.append(
                            f"{rel}:{lineno}: missing anchor #{anchor} in `{span}`"
                        )

    print(f"checked {checked} intra-repo references in Markdown")
    print(f"external URLs skipped (not fetched): {external}")
    print(f"fenced sample paths not in the tree: {len(examples)}")
    if args.show_examples:
        for item in examples:
            print(f"example {item}")
    if errors:
        print(f"broken: {len(errors)}")
        for item in errors:
            print(item)
        return 1
    print("broken: 0")
    return 0


if __name__ == "__main__":
    sys.exit(main())
