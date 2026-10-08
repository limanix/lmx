#!/usr/bin/env python3
"""Prepare lmx Markdown for the LimaNix documentation site."""

from __future__ import annotations

import argparse
import re
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE_URL = "https://github.com/limanix/lmx/blob/{ref}/"
# A link that leaves guides/, such as ../contract/v1/status.json, names a repository file.
REPOSITORY_LINK = re.compile(r"\]\(\.\./([^)#\s]+)(#[^)\s]*)?\)")
REF = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)|[0-9a-f]{40}")


def read_guides(root: Path) -> dict[Path, str]:
    """Read guide Markdown and reject links that copytree would follow."""
    guides = root / "guides"
    if guides.is_symlink() or not guides.is_dir():
        raise ValueError("guides/ must be a directory")
    if not (guides / "index.md").is_file():
        raise ValueError("Missing guide: guides/index.md")
    documents = {}
    for source in sorted(guides.rglob("*")):
        if source.is_symlink():
            raise ValueError(f"guide source must not be a symlink: {source}")
        if source.is_file() and source.suffix == ".md":
            documents[source.relative_to(guides)] = source.read_text(encoding="utf-8")
    return documents


def prepare(root: Path, ref: str) -> Path:
    if not REF.fullmatch(ref):
        raise ValueError(
            "LMX_REF must be a release tag such as v0.1.0 or a full commit SHA"
        )

    guides = read_guides(root)
    output = root / "build" / "docs"
    if output.parent.is_symlink() or output.is_symlink():
        raise ValueError("build/ and build/docs/ must not be symlinks")
    if output.exists():
        shutil.rmtree(output)
    shutil.copytree(root / "guides", output)

    source_url = SOURCE_URL.format(ref=ref)
    for relative, text in guides.items():
        text = REPOSITORY_LINK.sub(
            lambda match: f"]({source_url}{match[1]}{match[2] or ''})", text
        )
        (output / relative).write_text(text, encoding="utf-8")
    return output


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root", type=Path, default=ROOT, help="lmx repository directory"
    )
    parser.add_argument(
        "--ref", required=True, help="Release tag or commit SHA for source links"
    )
    args = parser.parse_args(argv)
    try:
        output = prepare(args.root, args.ref)
    except ValueError as error:
        print(f"docs/prepare: {error}", file=sys.stderr)
        return 2
    except OSError as error:
        print(f"docs/prepare: {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("docs/prepare: interrupted", file=sys.stderr)
        return 130
    print(f"Prepared documentation in {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
