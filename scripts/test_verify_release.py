#!/usr/bin/env python3
"""Exercise the verifier with a real native xtask release and corrupted copies."""
import argparse
import json
from pathlib import Path
import shutil
import tempfile

from verify_release import verify


def run(source):
    manifest = json.loads((source / "manifest.json").read_text())
    commit, target = manifest["git_commit"], manifest["target"]
    verify(source, commit, target)
    rejected = []
    for case in ["commit", "target", "archive_hash", "binary_hash", "checksums", "extra", "symlink"]:
        with tempfile.TemporaryDirectory(prefix="agend-release-adversary-") as temporary:
            out = Path(temporary) / "artifact"
            shutil.copytree(source, out)
            changed = dict(manifest)
            if case == "commit":
                changed["git_commit"] = "0" * 40
            elif case == "target":
                changed["target"] = "wrong-target"
            elif case == "archive_hash":
                changed["archive_sha256"] = "0" * 64
            elif case == "binary_hash":
                changed["binary_sha256"] = "0" * 64
            elif case == "checksums":
                (out / "SHA256SUMS").write_text("wrong\n")
            elif case == "extra":
                (out / "unexpected").touch()
            elif case == "symlink":
                (out / "SHA256SUMS").unlink()
                (out / "SHA256SUMS").symlink_to(source / "SHA256SUMS")
            (out / "manifest.json").write_text(json.dumps(changed))
            try:
                verify(out, commit, target)
            except ValueError:
                rejected.append(case)
            else:
                raise AssertionError(f"accepted corrupted release: {case}")
    print(json.dumps({"native_round_trip": "PASS", "rejected": rejected}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    run(parser.parse_args().directory.resolve())
