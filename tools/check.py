#!/usr/bin/env python3
"""Build HachiStep and run independent Hachiware guest diagnostics."""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

from _support import (
    ROOT, binary, create_directory, digest, release_executable, run,
    source_identity, versions, write_summary,
)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "out/check")
    parser.add_argument("--hachiware", type=Path, default=ROOT.parent / "hachiware")
    parser.add_argument("--case", action="append", default=[], metavar="GLOB")
    parser.add_argument("--runner", type=Path, help="use an existing native CLI")
    args = parser.parse_args()
    suite = args.hachiware.resolve()
    if not all((suite / name).is_file() for name in ("build.py", "run.py")):
        parser.error("clone lumirth/hachiware alongside this checkout or pass --hachiware PATH")
    if args.runner and (not args.runner.is_file() or args.runner.suffix == ".wasm"):
        parser.error("--runner requires a native CLI executable")
    source = source_identity()
    out = create_directory(args.out)
    cargo = binary("cargo")
    records = []
    runner = None
    unchanged = None
    failure = None
    try:
        steps = [
            ("format", [cargo, "fmt", "--all", "--", "--check"]),
            ("clippy", [cargo, "clippy", "--workspace", "--all-targets", "--all-features", "--locked", "--", "-D", "warnings"]),
        ]
        if not args.runner:
            steps.append(("build", [cargo, "build", "--workspace", "--release", "--locked"]))
        for name, command in steps:
            records.append(run(command, out, name, timeout=900))
        exe = args.runner.resolve() if args.runner else release_executable()
        runner = {"path": str(exe), "sha256": digest(exe)}
        selection = [part for pattern in args.case for part in ("--case", pattern)]
        records.append(run([sys.executable, str(suite / "build.py"), str(out / "fixtures"), *selection], out, "fixtures"))
        records.append(run([
            sys.executable, str(suite / "run.py"), "--fixtures", str(out / "fixtures"),
            "--adapter", str(ROOT / "tools/hachiware_adapter.py"), "--runner", str(exe),
            "--out", str(out / "conformance"),
        ], out, "conformance"))
        unchanged = runner["sha256"] == digest(exe)
        if not unchanged:
            raise RuntimeError("runner changed during execution")
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
        failure = str(error)
    write_summary(out / "summary.json", {
        "schema": 1, "selection": args.case or "all", "runner": runner,
        "runner_unchanged": unchanged, "failure": failure,
        "toolchain": versions(), "steps": records,
    }, source)
    if failure:
        raise RuntimeError(failure)
    print(f"Guest diagnostics passed. Results: {out}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError) as error:
        raise SystemExit(str(error))
