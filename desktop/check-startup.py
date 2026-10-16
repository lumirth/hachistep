#!/usr/bin/env python3
"""Open the desktop application on macOS or Linux and retain its startup log."""
import argparse
import os
from pathlib import Path
import signal
import subprocess
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("executable", type=Path)
args = parser.parse_args()
command = [str(args.executable.resolve())]
if sys.platform == "linux":
    command = ["xvfb-run", "-a", *command]
log = Path("out/startup.log")
log.parent.mkdir(parents=True, exist_ok=True)
with log.open("w") as output:
    process = subprocess.Popen(command, stdout=output, stderr=output, start_new_session=True)
    try:
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        else:
            raise SystemExit(f"Desktop exited during startup: {process.returncode}")
    finally:
        try:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=5)
        except ProcessLookupError:
            pass
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
print(log.read_text())
