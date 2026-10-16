#!/usr/bin/env python3
"""Capture a macOS stack sample of the CLI executing a supplied firmware session."""
import argparse
import hashlib
import json
from pathlib import Path
import select
import shutil
import subprocess
import sys

from _support import cli_command

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--runner", type=Path, default=ROOT / "target/release/hachistep")
    p.add_argument("--firmware", type=Path, required=True)
    p.add_argument("--eeprom", type=Path, required=True)
    p.add_argument("--input", type=Path)
    p.add_argument("--milliseconds", type=int, default=86_405_000)
    p.add_argument("--seconds", type=int, choices=range(2, 5), default=3)
    p.add_argument("--out", type=Path, required=True)
    args = p.parse_args()
    sampler = shutil.which("sample") if sys.platform == "darwin" else None
    if not sampler or args.runner.suffix == ".wasm":
        p.error("stack sampling requires macOS and a native CLI")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    command = cli_command(args.runner, ["run", "--firmware", str(args.firmware.resolve()),
                          "--eeprom", str(args.eeprom.resolve()), "--milliseconds", str(args.milliseconds),
                          *(["--input", str(args.input.resolve())] if args.input else [])])
    with (out / "stderr.log").open("w") as errors:
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=errors, text=True)
        try:
            if not select.select([process.stdout], [], [], 30)[0] or not process.stdout.readline():
                raise RuntimeError("CLI did not start the session")
            result = subprocess.run([sampler, str(process.pid), str(args.seconds), "1", "-file", str(out / "sample.txt")],
                                    capture_output=True, text=True, timeout=args.seconds + 30)
            (out / "sampler.log").write_text(result.stdout + result.stderr)
            if result.returncode or process.poll() is not None:
                raise RuntimeError("sampling failed or the session ended early; inspect logs and choose a longer horizon")
        finally:
            if process.poll() is None:
                process.terminate()
            process.communicate(timeout=10)
    (out / "profile.json").write_text(json.dumps({
        "mode": "profiling", "command": command, "seconds": args.seconds,
        "runner_sha256": hashlib.sha256(args.runner.read_bytes()).hexdigest(),
        "firmware_sha256": hashlib.sha256(args.firmware.read_bytes()).hexdigest(),
        "eeprom_sha256": hashlib.sha256(args.eeprom.read_bytes()).hexdigest(),
    }, indent=2) + "\n")
    print(out / "sample.txt")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        sys.exit(str(error))
