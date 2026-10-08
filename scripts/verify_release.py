#!/usr/bin/env python3
"""Verify a native xtask release before retaining it as an Actions artifact."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def verify(directory, commit, target):
    manifest = json.loads((directory / "manifest.json").read_text())
    if manifest.get("format") != 1 or manifest.get("git_commit") != commit:
        raise ValueError("wrong manifest format or source commit")
    if manifest.get("target") != target:
        raise ValueError("wrong native target")
    version = manifest["version"]
    if not isinstance(version, str) or not re.fullmatch(r"[0-9A-Za-z.+-]+", version):
        raise ValueError("invalid version")
    name = f"agend-{version}-{target}"
    archive = name + ".tar.gz"
    if manifest.get("archive") != archive:
        raise ValueError("unexpected archive name")
    if {p.name for p in directory.iterdir()} != {archive, "manifest.json", "SHA256SUMS"}:
        raise ValueError("unexpected output files")
    for entry in directory.iterdir():
        if entry.is_symlink() or not entry.is_file():
            raise ValueError("release outputs must be regular files")
    checksum = sha256((directory / archive).read_bytes())
    if manifest.get("archive_sha256") != checksum:
        raise ValueError("archive hash mismatch")
    if (directory / "SHA256SUMS").read_text() != f"{checksum}  {archive}\n":
        raise ValueError("checksum file mismatch")
    with tarfile.open(directory / archive, "r:gz") as tar:
        members = tar.getmembers()
        expected = {name, f"{name}/agend", f"{name}/README.md", f"{name}/LICENSE"}
        if len(members) != 4 or {m.name.rstrip('/') for m in members} != expected:
            raise ValueError("unexpected archive entries")
        for member in members:
            if member.name.rstrip('/') == name:
                if not member.isdir():
                    raise ValueError("payload root is not a directory")
            elif not member.isfile():
                raise ValueError("payload contains a link or special file")
        executable = tar.getmember(f"{name}/agend")
        if executable.mode != 0o755:
            raise ValueError("wrong executable mode")
        binary = tar.extractfile(executable).read()
    if sha256(binary) != manifest.get("binary_sha256"):
        raise ValueError("binary hash mismatch")
    # Never extract arbitrary member paths, and never inherit the user's HOME.
    with tempfile.TemporaryDirectory(prefix="agend-release-verify-") as temporary:
        root = Path(temporary)
        executable = root / "agend"
        executable.write_bytes(binary)
        executable.chmod(0o755)
        result = subprocess.run(
            [str(executable), "--version"], check=True, capture_output=True,
            text=True, timeout=30, cwd=root,
            env={"HOME": str(root), "AGEND_HOME": str(root / "home")},
        )
        if result.stdout.strip() != f"agend {version}":
            raise ValueError("extracted version mismatch")
    return manifest


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--target", required=True)
    args = parser.parse_args()
    print(json.dumps(verify(args.directory, args.commit, args.target), sort_keys=True))
