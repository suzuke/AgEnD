#!/usr/bin/env python3
"""Install the generated formula on a disposable Actions runner, then remove it."""
import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import uuid

from release_formula import generate
from verify_release import inspect_archive


def local_formula(artifacts, commit, formula):
    expected = generate(artifacts, commit)
    if formula.read_text() != expected:
        raise ValueError("formula differs from the verified matrix generator")
    for directory in artifacts.iterdir():
        record = json.loads((directory / "manifest.json").read_text())
        remote = (f'https://github.com/suzuke/AgEnD/releases/download/'
                  f'v{record["version"]}/{record["archive"]}')
        if expected.count(remote) != 1:
            raise ValueError("archive URL must occur exactly once")
        expected = expected.replace(remote, (directory / record["archive"]).resolve().as_uri())
    return expected


def run(args):
    # This command mutates the runner's Brew prefix. Never default to a user installation.
    if os.environ.get("GITHUB_ACTIONS") != "true" or not os.environ.get("RUNNER_TEMP"):
        raise RuntimeError("Brew smoke is restricted to disposable GitHub Actions runners")
    target = {
        ("Darwin", "arm64"): "aarch64-apple-darwin",
        ("Darwin", "x86_64"): "x86_64-apple-darwin",
        ("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
        ("Linux", "aarch64"): "aarch64-unknown-linux-gnu",
    }[(platform.system(), platform.machine())]
    formula = local_formula(args.artifacts, args.commit, args.formula)
    directories = [p for p in args.artifacts.iterdir()
                   if json.loads((p / "manifest.json").read_text())["target"] == target]
    manifest, _ = inspect_archive(directories[0], args.commit, target)
    brew = shutil.which("brew")
    if not brew:
        raise RuntimeError("runner has no Homebrew")
    with tempfile.TemporaryDirectory(prefix="agend-brew-", dir=os.environ["RUNNER_TEMP"]) as temporary:
        root = Path(temporary)
        env = dict(os.environ, HOMEBREW_NO_AUTO_UPDATE="1", HOMEBREW_NO_ANALYTICS="1",
                   HOMEBREW_NO_INSTALL_CLEANUP="1", HOMEBREW_NO_INSTALLED_DEPENDENTS_CHECK="1",
                   HOMEBREW_CACHE=str(root / "cache"), HOMEBREW_LOGS=str(root / "logs"),
                   HOMEBREW_TEMP=str(root / "tmp"))
        for name in ("cache", "logs", "tmp", "user"):
            (root / name).mkdir()

        def command(*argv):
            result = subprocess.run([brew, *argv], env=env, text=True, capture_output=True)
            print(result.stdout, end="", flush=True)
            print(result.stderr, end="", flush=True)
            result.check_returncode()
            return result.stdout.strip()

        prefix = Path(command("--prefix"))
        cellar = Path(command("--cellar")) / "agend"
        if any(os.path.lexists(p) for p in (cellar, prefix / "bin/agend", prefix / "opt/agend")):
            raise RuntimeError("refusing a runner with an existing agend installation")
        tap = f"agend-validation/smoke-{uuid.uuid4().hex}"
        name = tap + "/agend"
        repository = Path(command("--repository"))
        owner, repository_name = tap.split("/")
        tap_path = repository / "Library/Taps" / owner / ("homebrew-" + repository_name)
        if os.path.lexists(tap_path):
            raise RuntimeError("refusing an existing validation tap")
        try:
            command("tap-new", tap)
            if Path(command("--repo", tap)) != tap_path:
                raise RuntimeError("unexpected validation tap path")
            destination = tap_path / "Formula/agend.rb"
            with destination.open("x") as output:
                output.write(formula)
            command("install", "--formula", name)
            command("test", name)
            executable = Path(command("--prefix", name)) / "bin/agend"
            version = subprocess.check_output([str(executable), "--version"], text=True,
                env={"HOME": str(root / "user"), "PATH": "/usr/bin:/bin"}).strip()
            if version != f'agend {manifest["version"]}':
                raise RuntimeError("Brew installed version mismatch")
            init = subprocess.run([str(executable), "init"], text=True, capture_output=True,
                env={"HOME": str(root / "user"), "PATH": "/usr/bin:/bin"})
            home = root / "user/.agend"
            if init.returncode not in (0, 1) or not (home / "config.toml").is_file():
                raise RuntimeError("Brew installed init failed: " + init.stdout + init.stderr)
            if home.stat().st_mode & 0o777 != 0o700 or (home / "config.toml").stat().st_mode & 0o777 != 0o600:
                raise RuntimeError("Brew installed init permissions differ")
        finally:
            # The preflight proved this formula absent; do not touch other installed formulae.
            if cellar.exists():
                command("uninstall", "--formula", name)
            if tap_path.exists():
                command("untap", tap)
        if any(os.path.lexists(p) for p in (cellar, prefix / "bin/agend", prefix / "opt/agend", tap_path)):
            raise RuntimeError("Brew smoke left installation or tap resources")
    return {"status": "PASS", "source_commit": args.commit, "target": target,
            "scope": "generated formula with local archive URLs: install, test, fresh HOME init, uninstall",
            "public_release_urls_tested": False, "cleaned": True}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--formula", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    print(json.dumps(run(parser.parse_args()), indent=2))
