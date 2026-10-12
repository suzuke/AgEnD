#!/usr/bin/env python3
"""Run the real pipeline with an extracted release and explicit fake workers."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import time

from verify_release import inspect_archive


def run(args):
    started = time.monotonic()
    manifest, binary = inspect_archive(args.directory, args.commit, args.target)
    with tempfile.TemporaryDirectory(prefix="agend-install-extract-", dir="/tmp") as temporary:
        root = Path(temporary)
        executable = root / "agend"
        executable.write_bytes(binary)
        executable.chmod(0o755)
        result = subprocess.run(
            [str(args.probe.resolve()), "install"], capture_output=True, text=True,
            cwd=root, env={
                "HOME": str(root), "TMPDIR": "/tmp", "PATH": "/usr/bin:/bin",
                "AGEND_BIN": str(executable),
                "AGEND_WORKER_BIN": str(args.worker.resolve()),
            },
        )
        if result.returncode:
            raise RuntimeError(result.stdout + result.stderr)
        if "installed first task:" not in result.stdout:
            raise RuntimeError("probe omitted first-task evidence")
    elapsed = time.monotonic() - started
    if elapsed >= 300:
        raise RuntimeError(f"archive-to-task exceeded five minutes: {elapsed}")
    return {
        "status": "PASS", "archive_commit": manifest["git_commit"],
        "elapsed_seconds": round(elapsed, 3), "probe_output": result.stdout,
        "scope": "extracted release, fresh HOME, native pipeline with fake workers",
        "model_calls": 0, "extraction_cleaned": True,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--worker", type=Path, required=True)
    print(json.dumps(run(parser.parse_args()), indent=2))
