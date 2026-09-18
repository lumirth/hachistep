#!/usr/bin/env python3
"""Repeated end-to-end CLI measurements with raw samples and optional ABBA pairing.

External time includes output export and process launch. Each CLI report also
records wall time spent in the simulation loop.
"""

from __future__ import annotations

import argparse
import json
import random
import statistics
import subprocess
import time
from pathlib import Path

from _support import (
    ROOT,
    cli_command,
    create_directory,
    digest,
    environment,
    source_identity,
    versions,
    write_json,
)
from compare_runs import compare

ENDPOINT_KEYS = (
    "time_raw",
    "er",
    "ccr",
    "pc",
    "phase",
    "retired",
    "interrupt_entries",
    "ram_sha256",
    "lcd_ram_sha256",
    "eeprom_sha256",
    "eeprom_status",
    "events",
    "lcd_events",
    "nv_commits",
    "buzzer_events",
    "ir_events",
)


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--left", type=Path, default=ROOT / "target/release/hachistep")
    p.add_argument("--right", type=Path)
    p.add_argument("--firmware", type=Path, default=ROOT / "inputs/pokewalker.bin")
    p.add_argument("--eeprom", type=Path, default=ROOT / "inputs/eeprom.bin")
    p.add_argument("--input", type=Path)
    p.add_argument("--milliseconds", type=int, default=10000)
    p.add_argument(
        "--chunk-us",
        type=int,
        default=1000,
        help="execution horizon per core call in microseconds",
    )
    p.add_argument("--repeats", type=int, default=3)
    p.add_argument("--out", type=Path, default=ROOT / "out/benchmark")
    p.add_argument(
        "--preflight-trace-limit",
        type=int,
        default=2_000_000,
        help="bounded product-history capacity per untimed comparison run; truncation fails",
    )
    a = p.parse_args()
    if a.repeats < 2 or min(a.milliseconds, a.chunk_us, a.preflight_trace_limit) < 1:
        raise ValueError(
            "repeats must be >=2; duration, chunk and preflight trace limit must be positive"
        )
    out = create_directory(a.out)
    variants = {"left": a.left.resolve()}
    if a.right:
        variants["right"] = a.right.resolve()
    inputs = {"firmware": digest(a.firmware), "eeprom": digest(a.eeprom)}
    if a.input:
        inputs["timeline"] = digest(a.input)
    arguments = [
        "run",
        "--firmware",
        str(a.firmware.resolve()),
        "--eeprom",
        str(a.eeprom.resolve()),
        "--milliseconds",
        str(a.milliseconds),
        "--chunk-us",
        str(a.chunk_us),
    ]
    if a.input:
        arguments += ["--input", str(a.input.resolve())]
    equivalence = None
    if a.right:
        # Verify behavior before timing runs. Tracing and hashing product events
        # happen in this preflight so their cost stays outside the measurements.
        for variant, executable in variants.items():
            command = [
                *arguments,
                "--out",
                str(out / f"preflight-{variant}"),
                "--trace",
                str(out / f"preflight-{variant}.txt"),
                "--trace-limit",
                str(a.preflight_trace_limit),
            ]
            process = subprocess.run(
                cli_command(executable, command),
                capture_output=True,
                text=True,
                env=environment(),
                timeout=300,
                check=False,
            )
            (out / f"preflight-{variant}.log").write_text(
                process.stdout + process.stderr
            )
            if process.returncode:
                raise RuntimeError(f"{variant} preflight failed; inspect its log")
        equivalence = compare(
            out / "preflight-left",
            out / "preflight-right",
            out / "preflight-left.txt",
            out / "preflight-right.txt",
        )
        write_json(out / "equivalence.json", equivalence)
        if not equivalence["equivalent"]:
            raise RuntimeError(
                "observations differ: do not treat a behavior change as a performance-only optimization"
            )
        print(
            "Untimed exported-state and complete product-history comparison passed.",
            flush=True,
        )
    schedule = []
    rng = random.Random(0)
    for repeat in range(a.repeats):
        if a.right:
            order = ["left", "right", "right", "left"]
            if rng.randrange(2):
                order = ["right" if x == "left" else "left" for x in order]
        else:
            order = ["left"]
        schedule += [(repeat, variant) for variant in order]
    results, endpoint = [], None
    for index, (repeat, variant) in enumerate(schedule):
        destination = out / f"{index:03}-{variant}"
        command = [
            *arguments,
            "--out",
            str(destination),
        ]
        start = time.perf_counter()
        process = subprocess.run(
            cli_command(variants[variant], command),
            capture_output=True,
            text=True,
            env=environment(),
            timeout=300,
            check=False,
        )
        wall = time.perf_counter() - start
        (out / f"{index:03}.log").write_text(process.stdout + process.stderr)
        if process.returncode:
            raise RuntimeError(f"runner failed: {process.stderr}")
        report = json.loads((destination / "report.json").read_text())
        current = {k: report[k] for k in ENDPOINT_KEYS}
        if endpoint is None:
            endpoint = current
        elif endpoint != current:
            raise RuntimeError(
                "endpoints differ: reject performance comparison and inspect the reports"
            )
        record = {
            "index": index,
            "repeat": repeat,
            "variant": variant,
            "process_wall_seconds": wall,
            "simulation_wall_seconds": report["wall_seconds"],
        }
        results.append(record)
        print(json.dumps(record), flush=True)
    summaries = {}
    for variant in variants:
        timings = [
            r["simulation_wall_seconds"] for r in results if r["variant"] == variant
        ]
        summaries[variant] = {
            "samples": len(timings),
            "median_seconds": statistics.median(timings),
            "min_seconds": min(timings),
            "max_seconds": max(timings),
        }
    write_json(
        out / "summary.json",
        {
            "schema": 1,
            "source": source_identity(),
            "inputs": inputs,
            "toolchain": versions(),
            "chunk_us": a.chunk_us,
            "binaries": {k: digest(v) for k, v in variants.items()},
            "milliseconds": a.milliseconds,
            "samples": results,
            "summary": summaries,
            "endpoint": endpoint,
            "untimed_equivalence": equivalence,
            "limitation": "Untimed preflight compares native causal state, exported observations and complete product history. It is not physical-hardware conformance. Measured runs compare endpoints/counts without tracing.",
        },
    )


if __name__ == "__main__":
    try:
        main()
    except (
        OSError,
        ValueError,
        RuntimeError,
        KeyError,
        subprocess.SubprocessError,
    ) as e:
        raise SystemExit(str(e))
