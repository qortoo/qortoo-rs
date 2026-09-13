#!/usr/bin/env python3
"""Check relative Markdown links, in-file anchors, and the docs/ index for orphans.

Scope is deliberately narrow: local paths and local anchors only. External URLs
(http://, https://, mailto:, etc.) are never touched — this script makes no network
requests, so it can run in CI (or anywhere) without depending on outbound access or
being flaky because some third-party site was briefly down. Checking that external
URLs still resolve is a different, network-dependent problem left for later tooling.

Usage: python3 scripts/check_doc_links.py
Exit status is non-zero if any broken link, broken anchor, or orphaned/missing doc
is found; every finding is printed to stderr first.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DOCS_DIR = REPO_ROOT / "docs"

# Root-level Markdown files that participate in the same link graph as docs/.
ROOT_MD_FILES = ["README.md", "AGENTS.md", "CONTRIBUTING.md"]

LINK_RE = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
HEADING_RE = re.compile(r"^(#{1,6})\s+(.+?)\s*$")
EXTERNAL_SCHEMES = ("http://", "https://", "mailto:")


def slugify(heading: str) -> str:
    """Approximate GitHub's heading-to-anchor slugification."""
    # Strip inline code/emphasis markers and links' bracket text is left as-is;
    # GitHub keeps link/code text, so we only strip Markdown's own punctuation.
    text = re.sub(r"[`*_]", "", heading)
    text = text.lower()
    text = re.sub(r"[^\w\s-]", "", text)  # drop punctuation
    text = re.sub(r"\s+", "-", text.strip())
    return text


def headings_in(path: Path) -> set[str]:
    slugs: set[str] = set()
    seen: dict[str, int] = {}
    in_fence = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip().startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        m = HEADING_RE.match(line)
        if not m:
            continue
        base = slugify(m.group(2))
        # GitHub disambiguates repeated headings with -1, -2, ...
        n = seen.get(base, 0)
        seen[base] = n + 1
        slugs.add(base if n == 0 else f"{base}-{n}")
    return slugs


def find_markdown_files() -> list[Path]:
    files = [DOCS_DIR / name for name in sorted(p.name for p in DOCS_DIR.glob("*.md"))]
    files += [REPO_ROOT / name for name in ROOT_MD_FILES if (REPO_ROOT / name).exists()]
    return files


def check_links(files: list[Path]) -> list[str]:
    errors: list[str] = []
    for md_file in files:
        text = md_file.read_text(encoding="utf-8")
        in_fence = False
        for lineno, line in enumerate(text.splitlines(), start=1):
            if line.strip().startswith("```"):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            for match in LINK_RE.finditer(line):
                target = match.group(1).strip()
                if any(target.startswith(scheme) for scheme in EXTERNAL_SCHEMES):
                    continue
                file_part, _, anchor = target.partition("#")
                if file_part == "":
                    # Pure in-page anchor, e.g. [x](#some-heading).
                    resolved = md_file
                else:
                    resolved = (md_file.parent / file_part).resolve()
                    if not resolved.exists() or not resolved.is_file():
                        errors.append(
                            f"{md_file.relative_to(REPO_ROOT)}:{lineno}: "
                            f"broken link target '{file_part}' (resolved to {resolved})"
                        )
                        continue
                if anchor and resolved.suffix == ".md":
                    if anchor not in headings_in(resolved):
                        errors.append(
                            f"{md_file.relative_to(REPO_ROOT)}:{lineno}: "
                            f"anchor '#{anchor}' not found in {resolved.relative_to(REPO_ROOT)}"
                        )
    return errors


def check_orphans() -> list[str]:
    """Every docs/*.md file except the index itself must be linked from docs/README.md."""
    index = DOCS_DIR / "README.md"
    index_text = index.read_text(encoding="utf-8")
    linked = {m.group(1) for m in LINK_RE.finditer(index_text) if m.group(1).endswith(".md")}
    errors = []
    for md_file in sorted(DOCS_DIR.glob("*.md")):
        if md_file.name == "README.md":
            continue
        if md_file.name not in linked:
            errors.append(f"docs/{md_file.name} is not linked from docs/README.md's index")
    return errors


def main() -> int:
    files = find_markdown_files()
    errors = check_links(files) + check_orphans()
    if errors:
        print(f"{len(errors)} documentation link issue(s) found:", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        return 1
    print(f"OK: {len(files)} Markdown files, no broken local links, anchors, or orphans.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
