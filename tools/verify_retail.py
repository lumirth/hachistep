#!/usr/bin/env python3
"""Run private-input replay checks and real firmware workloads; never update inputs."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from _support import (
    ROOT,
    binary,
    cli_command,
    create_directory,
    digest,
    environment,
    release_executable,
    run,
    source_identity,
    versions,
    write_summary,
)
from compare_runs import compare_resume
from retail_expectations import check as check_expectations
from retail_expectations import load as load_expectations


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--firmware", type=Path, default=ROOT / "inputs/pokewalker.bin")
    p.add_argument("--eeprom", type=Path, default=ROOT / "inputs/eeprom.bin")
    p.add_argument("--out", type=Path, default=ROOT / "out/retail-check")
    p.add_argument(
        "--runner",
        type=Path,
        help="use a prebuilt native executable or WASI module for workloads",
    )
    selection = p.add_mutually_exclusive_group()
    selection.add_argument(
        "--quick",
        action="store_true",
        help="run the short boot, menu and settings workloads",
    )
    selection.add_argument(
        "--case", action="append", help="run a named workload; repeat to select several"
    )
    p.add_argument(
        "--list",
        action="store_true",
        help="list selected workloads without private inputs",
    )
    p.add_argument(
        "--menu-trace",
        action="store_true",
        help="retain a complete product trace for audio rendering",
    )
    p.add_argument("--expected", type=Path, default=ROOT / "workloads/retail.json")
    p.add_argument(
        "--smoke-only",
        action="store_true",
        help="check execution only; do not claim a behavioral regression match",
    )
    a = p.parse_args()
    firmware, eeprom = a.firmware.resolve(), a.eeprom.resolve()
    before = (
        None if a.list else {"firmware": digest(firmware), "eeprom": digest(eeprom)}
    )
    scenarios = load_expectations(a.expected, None if a.smoke_only else before)
    expected = None if a.smoke_only else scenarios
    if a.case and set(a.case) - scenarios["cases"].keys():
        raise ValueError(
            "unknown workload: "
            + ", ".join(sorted(set(a.case) - scenarios["cases"].keys()))
        )
    cases = {
        name: case
        for name, case in scenarios["cases"].items()
        if (not a.quick or case["quick"]) and (not a.case or name in a.case)
    }
    if a.list:
        for name, case in cases.items():
            print(
                f"{name}: {case['milliseconds']} ms; {case['timeline'] or 'no external input'}"
            )
        return
    source = source_identity()
    out = create_directory(a.out)
    cargo = binary("cargo")
    env = environment()
    env.update(HS_FIRMWARE=str(firmware), HS_EEPROM=str(eeprom))
    records = []
    for test, name in [("retail", "partition-and-snapshot"), ("retail_link", "peer-exchange")]:
        records.append(
            run(
                [
                    cargo,
                    "test",
                    "-p",
                    "hs-core",
                    "--release",
                    "--test",
                    test,
                    "--locked",
                    "--offline",
                    "--",
                    "--ignored",
                    "--nocapture",
                ],
                out,
                name,
                env,
            )
        )
    if a.runner is None:
        records.append(
            run(
                [cargo, "build", "-p", "hs-cli", "--release", "--locked", "--offline"],
                out,
                "build",
            )
        )
    exe = a.runner.resolve() if a.runner else release_executable()

    def execute(name, milliseconds, timeline, *, restore=None, trace=False):
        destination = out / name
        arguments = (
            ["--load-state", str(restore)]
            if restore
            else ["--firmware", str(firmware), "--eeprom", str(eeprom)]
        )
        command = [
            "run",
            *arguments,
            "--milliseconds",
            str(milliseconds),
            "--out",
            str(destination),
        ]
        if timeline:
            command += ["--input", str(timeline)]
        if trace:
            command += [
                "--trace",
                str(destination / "events.txt"),
                "--trace-limit",
                "2000000",
            ]
        records.append(run(cli_command(exe, command), out, name))
        return destination

    summaries = []
    mismatches = []
    for name, case in cases.items():
        ms = case["milliseconds"]
        timeline = (
            a.expected.resolve().parent / case["timeline"] if case["timeline"] else None
        )
        checkpoint = case.get("checkpoint_ms")
        directory = execute(
            name,
            ms,
            timeline,
            trace=bool(checkpoint) or (name == "menu" and a.menu_trace),
        )
        report = json.loads((directory / "report.json").read_text())
        if (
            report["fault"] is not None
            or report["requested_time_raw"] != report["time_raw"]
        ):
            raise RuntimeError(f"{name} did not reach its requested exclusive horizon")
        if report["serial_tx"] == 0 or report["interrupt_entries"] == 0:
            raise RuntimeError(
                f"{name} did not exercise the expected boot/peripheral integration"
            )
        differences = (
            []
            if expected is None
            else check_expectations(
                expected["cases"][name], ms, timeline, out / name, report
            )
        )
        mismatches += [f"{name}: {difference}" for difference in differences]
        replay = None
        if checkpoint:
            prefix = execute(name + "-before", checkpoint, timeline, trace=True)
            resumed = execute(
                name + "-resumed",
                ms,
                timeline,
                restore=prefix / "state.bin",
                trace=True,
            )
            replay = compare_resume(directory, prefix, resumed)
            mismatches += [
                f"{name}: {difference}" for difference in replay["differences"]
            ]
        summaries.append(
            {
                "regression_checked": expected is not None,
                "regression_differences": differences,
                "name": name,
                "milliseconds": ms,
                "restoration": replay,
                "frame_sha256": digest(out / name / "frame.pgm"),
                "report": report,
            }
        )
    after = {"firmware": digest(firmware), "eeprom": digest(eeprom)}
    if before != after:
        raise RuntimeError("source input changed")
    write_summary(
        out / "summary.json",
        {
            "schema": 1,
            "kind": "emulator observations, not hardware conformance",
            "toolchain": versions(),
            "runner_sha256": digest(exe),
            "inputs": before,
            "inputs_unchanged": True,
            "regression_checked": expected is not None,
            "regression_passed": not mismatches if expected else None,
            "expectation_file_sha256": digest(a.expected) if expected else None,
            "basis_revision": expected["basis_revision"] if expected else None,
            "workloads": summaries,
            "commands": records,
        },
        source,
    )
    if mismatches:
        raise RuntimeError(
            "Regression differences (not automatically emulator bugs):\n"
            + "\n".join(mismatches)
        )
    print(
        f"Retail {'software-regression' if expected else 'execution-only smoke'} passed; source images unchanged. Results: {out}"
    )


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, KeyError) as e:
        raise SystemExit(str(e))
