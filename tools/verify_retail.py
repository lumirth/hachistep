#!/usr/bin/env python3
"""Run firmware sessions and compare captured continuations with uninterrupted execution."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path

from _support import (
    ROOT, binary, cli_command, create_directory, digest, release_executable,
    run, source_identity, versions, write_summary,
)
from compare_runs import compare, compare_resume


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--firmware", type=Path, default=ROOT / "inputs/pokewalker.bin")
    p.add_argument("--eeprom", type=Path, default=ROOT / "inputs/eeprom.bin")
    p.add_argument("--out", type=Path, default=ROOT / "out/retail-check")
    p.add_argument("--runner", type=Path, help="native CLI or WASI module")
    selection = p.add_mutually_exclusive_group()
    selection.add_argument("--quick", action="store_true", help="home, menu, and settings")
    selection.add_argument("--case", action="append", help="select a session; repeatable")
    p.add_argument("--list", action="store_true")
    p.add_argument("--play", action="store_true", help="also run the native gameplay session")
    p.add_argument("--link", action="store_true", help="also run the native two-walker exchange")
    p.add_argument("--chunk-us", type=int, help="also compare short host calls")
    p.add_argument("--scenarios", type=Path, default=ROOT / "workloads/retail.json")
    a = p.parse_args()
    if a.chunk_us is not None and a.chunk_us < 1:
        p.error("--chunk-us must be positive")
    scenarios = json.loads(a.scenarios.read_text())["cases"]
    unknown = set(a.case or ()) - scenarios.keys()
    if unknown:
        p.error("unknown session: " + ", ".join(sorted(unknown)))
    cases = {name: case for name, case in scenarios.items()
             if (not a.quick or case["quick"]) and (not a.case or name in a.case)}
    if a.list:
        for name, case in cases.items():
            print(f"{name}: {case['milliseconds']} ms; {case['timeline'] or 'no external input'}")
        return
    firmware, eeprom = a.firmware.resolve(), a.eeprom.resolve()
    inputs = {"firmware": digest(firmware), "eeprom": digest(eeprom)}
    source = source_identity()
    out = create_directory(a.out)
    records, summaries, differences = [], [], []
    runner = None
    failure = None
    unchanged = None
    try:
        if a.runner is None:
            records.append(run([binary("cargo"), "build", "-p", "hs-cli", "--release", "--locked"], out, "build", timeout=900))
        exe = a.runner.resolve() if a.runner else release_executable()
        runner = {"path": str(exe), "sha256": digest(exe)}

        def execute(name, milliseconds, timeline, *, restore=None, trace=False, chunk=None):
            destination = out / name
            initial = (["--load-state", str(restore)] if restore else
                       ["--firmware", str(firmware), "--eeprom", str(eeprom)])
            command = ["run", *initial, "--milliseconds", str(milliseconds), "--out", str(destination)]
            if timeline:
                command += ["--input", str(timeline)]
            if trace:
                command += ["--trace", str(destination / "events.txt"), "--trace-limit", "2000000"]
            if chunk:
                command += ["--chunk-us", str(chunk)]
            records.append(run(cli_command(exe, command), out, name, timeout=900))
            report = json.loads((destination / "report.json").read_text())
            if report["fault"] is not None or report["requested_time_raw"] != report["time_raw"]:
                raise RuntimeError(f"{name}: fault or incomplete session")
            return destination, report

        for name, case in cases.items():
            ms = case["milliseconds"]
            timeline = a.scenarios.resolve().parent / case["timeline"] if case["timeline"] else None
            checkpoint = case.get("checkpoint_ms")
            directory, report = execute(name, ms, timeline, trace=bool(checkpoint or a.chunk_us))
            restoration = partition = None
            if checkpoint:
                prefix, _ = execute(name + "-before", checkpoint, timeline, trace=True)
                resumed, _ = execute(name + "-resumed", ms, timeline, restore=prefix / "state.bin", trace=True)
                restoration = compare_resume(directory, prefix, resumed)
                differences += [f"{name}: {d}" for d in restoration["differences"]]
            if a.chunk_us:
                chunked, _ = execute(name + "-chunked", ms, timeline, trace=True, chunk=a.chunk_us)
                partition = compare(directory, chunked, directory / "events.txt", chunked / "events.txt")
                differences += [f"{name}: {d}" for d in partition["differences"]]
            summaries.append({"name": name, "milliseconds": ms, "restoration": restoration,
                              "partition": partition, "report": report})
        if a.play or a.link:
            for name in ("play", "link"):
                if getattr(a, name):
                    records.append(run([
                        binary("cargo"), "run", "-p", "hs-core", "--release", "--locked",
                        "--example", "retail", "--", name, str(firmware), str(eeprom),
                    ], out, name, timeout=900))
        unchanged = runner["sha256"] == digest(exe)
        if not unchanged:
            raise RuntimeError("runner changed during execution")
        if inputs != {"firmware": digest(firmware), "eeprom": digest(eeprom)}:
            raise RuntimeError("input images changed during execution")
        if differences:
            raise RuntimeError("\n".join(differences))
    except (OSError, RuntimeError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
        failure = str(error)
    write_summary(out / "summary.json", {
        "schema": 1, "toolchain": versions(), "runner": runner,
        "runner_unchanged": unchanged, "inputs": inputs,
        "workloads": summaries, "commands": records, "failure": failure,
    }, source)
    if failure:
        raise RuntimeError(failure)
    print(f"Firmware sessions passed. Results: {out}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, KeyError) as error:
        raise SystemExit(str(error))
