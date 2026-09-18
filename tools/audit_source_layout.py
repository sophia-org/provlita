#!/usr/bin/env python3
"""Check Provlita-owned source lengths in the tracked and unignored working tree."""

import argparse
import json
from pathlib import Path
import subprocess
import sys


REVIEW_LINES = 800
MAX_SOURCE_LINES = 1000
SOURCE_SUFFIXES = frozenset(
    {".rs", ".py", ".sh", ".c", ".h", ".cc", ".cpp", ".hpp", ".nim", ".zig",
     ".wgsl", ".glsl", ".vert", ".frag", ".comp"}
)
EXTERNAL_ROOTS = frozenset({"vendor", "third_party", "target", ".artifacts"})


def source_paths(root):
    result = subprocess.run(
        ["git", "-C", str(root), "ls-files", "--cached", "--others",
         "--exclude-standard", "-z"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    paths = {Path(item.decode("utf-8", errors="surrogateescape"))
             for item in result.stdout.split(b"\0") if item}
    return sorted(
        path for path in paths
        if path.parts[0] not in EXTERNAL_ROOTS
        and path.suffix.lower() in SOURCE_SUFFIXES
    )


def is_test(path):
    directories = path.parts[:-1]
    if "tests" not in directories:
        return False
    # src/tests is still production-tree debt, not a way around the limit.
    return "src" not in directories or directories.index("tests") < directories.index("src")


def audit(root):
    files = reports = errors = 0
    for relative in source_paths(root):
        path = root / relative
        label = json.dumps(relative.as_posix(), ensure_ascii=True)
        if path.is_symlink():
            print(f"error: source symlink is not audited: {label}", file=sys.stderr)
            errors += 1
            continue
        try:
            with path.open("rb") as source:
                lines = sum(1 for _ in source)
        except FileNotFoundError:
            # A tracked file removed from the working tree has no content to audit.
            continue
        files += 1
        test = is_test(relative)
        if lines >= REVIEW_LINES:
            category = "test-lines" if test else "source-lines"
            print(f"{category} {lines} {label}")
            reports += 1
        if not test and lines > MAX_SOURCE_LINES:
            print(f"error: {label} has {lines} lines; maximum is {MAX_SOURCE_LINES}",
                  file=sys.stderr)
            errors += 1
    print(f"source-layout files={files} reports={reports} errors={errors}")
    return 1 if errors else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1],
                        help="repository root; defaults to the tool's repository")
    args = parser.parse_args()
    try:
        return audit(args.root.resolve())
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"error: source-layout audit could not complete ({type(error).__name__})",
              file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
