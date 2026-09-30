#!/usr/bin/env python3
"""Save raw macOS samples of sustained workload_probe replays.

This is untimed profiling. Fresh-machine construction is included in the sample;
no replay wall time is presented as a performance measurement. Use an ordinary
uninstrumented adapter for host-cost sampling and compare_workloads.py for timing.
"""

from __future__ import annotations

import argparse
import json
import platform
import select
import shutil
import subprocess
from pathlib import Path

from _support import create_directory, digest, environment, run, source_identity, write_summary


def sample_case(adapter: Path, case: dict, seconds: int, sampler: str, directory: Path) -> dict:
    directory.mkdir()
    command = [
        str(adapter), case["mode"], str(case["rom"]), str(case["quantum_us"]),
        str(case["limit_us"]), "1", "--profile-seconds", str(seconds + 5),
    ]
    with (directory / "stderr.txt").open("w") as errors:
        process = subprocess.Popen(
            command, stdout=subprocess.PIPE, stderr=errors, text=True, env=environment(),
        )
        try:
            readable, _, _ = select.select([process.stdout], [], [], 30)
            if not readable:
                raise RuntimeError("profiling adapter did not announce readiness")
            ready = process.stdout.readline()
            readiness = json.loads(ready)
            if readiness.get("profile_ready") is not True:
                raise ValueError("adapter lacks explicit untimed profiling replay support")
            if readiness.get("instrumented") is not False:
                raise ValueError("sampling requires an uninstrumented profiling adapter")
            sampled = run(
                [sampler, str(process.pid), str(seconds), "1", "-file", str(directory / "sample.txt")],
                directory, "sample", timeout=seconds + 30,
            )
            remaining, _ = process.communicate(timeout=seconds + 30)
            (directory / "stdout.txt").write_text(ready + remaining)
            if process.returncode:
                raise RuntimeError(f"profiling adapter failed: {directory}")
            result = json.loads(remaining)
            if result.get("mode") != "profiling" or "ns" in result or "work" in result:
                raise ValueError("sampling reply must be explicitly untimed and uninstrumented")
            return {"case": {**case, "rom": str(case["rom"])}, "command": command, "sample": sampled, "reply": result}
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            if process.stdout:
                process.stdout.close()


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--adapter", type=Path, required=True)
    parser.add_argument("--rom-dir", type=Path, required=True)
    parser.add_argument("--case", action="append", default=[], help="ROM stem; default all board_*.bin")
    parser.add_argument("--quantum-us", type=int, default=1000)
    parser.add_argument("--seconds", type=int, default=3, help="macOS sample duration per case (2–4 seconds)")
    parser.add_argument("--build-receipt", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    if platform.system() != "Darwin" or not (sampler := shutil.which("sample")):
        parser.error("raw sampling requires the macOS sample tool")
    if args.seconds not in range(2, 5) or args.quantum_us < 1:
        parser.error("seconds must be 2–4 and quantum positive")
    adapter = args.adapter.resolve()
    roms = {path.stem: path.resolve() for path in sorted(args.rom_dir.glob("board_*.bin"))}
    chosen = args.case or list(roms)
    if not chosen or set(chosen) - roms.keys():
        parser.error("no workloads or an unknown case selected")
    source = source_identity()
    out = create_directory(args.out)
    receipt = None
    if args.build_receipt:
        receipt = {"path": str(args.build_receipt.resolve()), "sha256": digest(args.build_receipt)}
        (out / "build-receipt.txt").write_bytes(args.build_receipt.read_bytes())
    report = {
        "scope": "untimed fresh-machine replays including construction; raw stack samples, no speed acceptance",
        "platform": platform.platform(), "adapter": {"path": str(adapter), "sha256": digest(adapter)},
        "sampler": {"path": sampler, "sha256": digest(Path(sampler))},
        "build_receipt": receipt, "roms": {name: {"path": str(roms[name]), "sha256": digest(roms[name])} for name in chosen},
        "results": [],
    }
    try:
        for index, name in enumerate(chosen):
            case = {
                "name": name, "rom": roms[name], "quantum_us": args.quantum_us,
                "mode": "time" if "idle" in name else "job",
                "limit_us": 1_000_000 if "idle" in name else 1_000_000_000,
            }
            result = sample_case(adapter, case, args.seconds, sampler, out / f"{index:03}-{name}")
            report["results"].append(result)
            print(f"{name}: raw sample saved", flush=True)
    except Exception as error:
        report["failure"] = str(error)
        raise
    finally:
        report["inputs_unchanged"] = (
            digest(adapter) == report["adapter"]["sha256"]
            and all(digest(roms[name]) == report["roms"][name]["sha256"] for name in chosen)
        )
        write_summary(out / "summary.json", report, source)
    if not report["inputs_unchanged"]:
        raise RuntimeError("adapter/ROM changed during profiling; summary retained")


if __name__ == "__main__":
    main()
