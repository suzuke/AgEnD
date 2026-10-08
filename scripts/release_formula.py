#!/usr/bin/env python3
"""Render a reviewable Brew formula from four verified native release archives."""
import argparse
from pathlib import Path
import re

from verify_release import inspect_archive


TARGETS = {
    "aarch64-apple-darwin": ("macos", "arm"),
    "x86_64-apple-darwin": ("macos", "intel"),
    "aarch64-unknown-linux-gnu": ("linux", "arm"),
    "x86_64-unknown-linux-gnu": ("linux", "intel"),
}


def generate(root, commit):
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("expected an exact lowercase source commit")
    records = {}
    for directory in sorted(root.iterdir()):
        if not directory.is_dir() or directory.is_symlink():
            raise ValueError("expected only native artifact directories")
        # The target comes from the manifest, but is checked against a fixed allowlist.
        import json
        target = json.loads((directory / "manifest.json").read_text())["target"]
        if target not in TARGETS or target in records:
            raise ValueError("unknown or duplicate target")
        records[target], _ = inspect_archive(directory, commit, target)
    if set(records) != set(TARGETS):
        raise ValueError("all four native targets are required")
    versions = {record["version"] for record in records.values()}
    if len(versions) != 1:
        raise ValueError("release versions differ")
    version = versions.pop()
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", version):
        raise ValueError("version is not suitable for a release tag")
    lines = [
        f"# Generated from source {commit}; review before publishing.",
        "class Agend < Formula",
        '  desc "Daemon for coordinating coding agents"',
        '  homepage "https://github.com/suzuke/AgEnD"',
        f'  version "{version}"',
        '  license "Apache-2.0"',
        "",
    ]
    for platform in ("macos", "linux"):
        lines.append(f"  on_{platform} do")
        for target, (os_name, arch) in TARGETS.items():
            if os_name != platform:
                continue
            record = records[target]
            lines += [
                f"    on_{arch} do",
                f'      url "https://github.com/suzuke/AgEnD/releases/download/v{version}/{record["archive"]}"',
                f'      sha256 "{record["archive_sha256"]}"',
                "    end",
            ]
        lines += ["  end", ""]
    lines += [
        "  def install", '    bin.install "agend"', "  end", "",
        "  test do",
        '    assert_equal "agend #{version}", shell_output("#{bin}/agend --version").strip',
        "  end", "end", "",
    ]
    return "\n".join(lines)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    formula = generate(args.artifacts, args.commit)
    # Never replace a reviewed formula or publish to a tap implicitly.
    with args.out.open("x") as output:
        output.write(formula)
