#!/usr/bin/env python3
"""Render every Mermaid diagram in docs/*.md and fail on the first syntax error.

This only checks that Mermaid can parse and render each diagram — it says nothing
about whether the diagram's labels still match the code (that's a manual
code-comparison job, done once per document when it's written or revised, not
something a renderer can verify). Requires Node/npm; downloads
@mermaid-js/mermaid-cli via npx on first run, which needs network access to the
npm registry — unlike check_doc_links.py, this script is not offline-safe.

Usage: python3 scripts/check_mermaid.py
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DOCS_DIR = REPO_ROOT / "docs"
MERMAID_BLOCK_RE = re.compile(r"```mermaid\n(.*?)```", re.DOTALL)


def main() -> int:
    if not shutil.which("npx"):
        print("error: npx not found; Node.js/npm is required to run this check", file=sys.stderr)
        return 1

    failures: list[str] = []
    total = 0
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        # mermaid-cli renders through headless Chromium via Puppeteer. Chromium's
        # own sandbox needs a container/kernel feature that CI runners (GitHub
        # Actions' ubuntu-latest included) commonly don't grant to an
        # unprivileged process, so it refuses to start at all without this flag
        # — this is Puppeteer's own documented workaround, not a project-specific
        # hack, and it only affects this render step, not any other sandboxing.
        puppeteer_config = tmp_path / "puppeteer-config.json"
        puppeteer_config.write_text('{"args": ["--no-sandbox"]}', encoding="utf-8")

        for md_file in sorted(DOCS_DIR.glob("*.md")):
            text = md_file.read_text(encoding="utf-8")
            for i, block in enumerate(MERMAID_BLOCK_RE.findall(text)):
                total += 1
                src = tmp_path / f"{md_file.stem}_{i}.mmd"
                out = tmp_path / f"{md_file.stem}_{i}.svg"
                src.write_text(block, encoding="utf-8")
                result = subprocess.run(
                    [
                        "npx", "--yes", "@mermaid-js/mermaid-cli",
                        "-i", str(src), "-o", str(out),
                        "-p", str(puppeteer_config),
                    ],
                    capture_output=True,
                    text=True,
                )
                if result.returncode != 0 or not out.exists():
                    failures.append(
                        f"{md_file.relative_to(REPO_ROOT)} (diagram #{i}): "
                        f"{result.stderr.strip() or result.stdout.strip()}"
                    )

    if failures:
        print(f"{len(failures)} of {total} Mermaid diagram(s) failed to render:", file=sys.stderr)
        for f in failures:
            print(f"  - {f}", file=sys.stderr)
        return 1
    print(f"OK: {total} Mermaid diagram(s) across docs/ rendered without error.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
