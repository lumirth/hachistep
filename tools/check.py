#!/usr/bin/env python3
"""Offline source, Rust, CLI safety and independent hachiware checks."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path

from _support import (
    ROOT,
    binary,
    create_directory,
    digest,
    release_executable,
    run,
    source_identity,
    versions,
    write_summary,
)

# Execution checks cover bus ordering, peripheral appointments, interrupt admission,
# host suspension and restored continuations. Full checks own the remaining tests.
EXECUTION_TESTS = (
    "cpu_regressions",
    "kernel",
    "register_access",
    "clock_obligations",
    "interrupt_admission",
    "interrupt_reads",
    "serial_appointments",
    "serial_observers",
    "reset_release",
    "watchdog",
    "sensor_spi",
    "sensor_i2c",
    "output_control",
    "save_state",
    "flash_execution",
    "allocation",
)
EXECUTION_CASES = (
    "register-native-word-byte-lanes",
    "register-holes-and-mixed-word",
    "execute-from-ram",
    "predecrement-alias-*",
    "prefetch-before-self-modifying-store",
    "call-prefetch-before-stack-write",
    "arithmetic-add-16-0",
    "exception-ccr-stack-word",
    "irq-enable-clear-admission",
    "irq-source-clear-cancels",
    "nmi-masked-sleep",
    "serial-eeprom-page-wrap",
    "infrared-transmit",
    "infrared-receive",
    "sci-overrun-retains-rdr",
    "sci-external-synchronous",
    "ssu-replace-queued-byte",
    "ssu-slave-deselect-in-frame",
    "flash-protection-selected",
)


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "out/check")
    parser.add_argument(
        "--hachiware",
        type=Path,
        default=ROOT.parent / "hachiware",
        help="separate hardware suite checkout (default: ../hachiware)",
    )
    parser.add_argument(
        "--execution",
        action="store_true",
        help="native execution iteration checks; omits the full validation matrices",
    )
    parser.add_argument(
        "--test", action="append", default=[], metavar="TARGET[::EXACT_TEST]",
        help="run only a core lib/integration target or exact test; repeat to combine",
    )
    parser.add_argument(
        "--case", action="append", default=[], metavar="GLOB",
        help="run only selected hachiware guests; repeat to combine",
    )
    parser.add_argument(
        "--features", help="selected Rust tests only: trace, profile-work, or both comma-separated",
    )
    parser.add_argument(
        "--runner",
        type=Path,
        help="use an existing native CLI for guest diagnostics; skip its build",
    )
    args = parser.parse_args(argv)
    focused = bool(args.test or args.case)
    if args.execution and (focused or args.features):
        parser.error("--execution cannot be combined with --test, --case or --features")
    if args.features and not args.test:
        parser.error("--features requires --test")
    if args.features and not set(args.features.split(",")) <= {"trace", "profile-work"}:
        parser.error("--features accepts trace and profile-work, comma-separated")
    if focused and args.runner and not args.case:
        parser.error("--runner requires --case in focused checks")
    targets = {path.stem for path in (ROOT / "crates/hs-core/tests").glob("*.rs")}
    selections = []
    for selection in dict.fromkeys(args.test):
        target, separator, exact = selection.partition("::")
        if target != "lib" and target not in targets:
            parser.error(f"unknown core test target: {target}")
        if separator and not exact:
            parser.error(f"missing exact test name: {selection}")
        selections.append((target, exact))
    suite = args.hachiware.resolve()
    guests = not focused or bool(args.case)
    if guests and (not (suite / "build.py").is_file() or not (suite / "run.py").is_file()):
        raise RuntimeError(
            "hachiware checkout required: gh repo clone lumirth/hachiware ../hachiware; "
            "or pass --hachiware PATH"
        )
    supplied = args.runner.resolve() if args.runner else None
    if supplied and (not supplied.is_file() or supplied.suffix == ".wasm"):
        parser.error("--runner must name an existing native CLI executable")
    needs_cargo = not focused or bool(selections) or (guests and not supplied)
    cargo = binary("cargo") if needs_cargo else None
    source = source_identity()
    out = create_directory(args.out)
    records = []
    profile = "iteration" if args.execution or focused else "release"
    offline = ["--locked", "--offline"]
    steps = []
    rust_checks = {}
    if focused and selections:
        base = [cargo, "test", "-p", "hs-core", "--profile", profile, *offline]
        if args.features:
            base += ["--features", args.features]
        whole = {target for target, exact in selections if not exact}
        if whole:
            command = base.copy()
            for target in sorted(whole):
                command += ["--lib"] if target == "lib" else ["--test", target]
            steps.append(("focused-tests", command))
            rust_checks["focused-tests"] = len(whole)
        for index, (target, exact) in enumerate(selections):
            if exact and target not in whole:
                name = f"focused-test-{index + 1}"
                target_args = ["--lib"] if target == "lib" else ["--test", target]
                command = [*base, *target_args,
                           "--", exact, "--exact"]
                steps.append((name, command))
                rust_checks[name] = 1
    elif not focused:
        steps.append(("format", [cargo, "fmt", "--all", "--", "--check"]))
    if args.execution:
        tests = [cargo, "test", "-p", "hs-core", "--profile", profile, "--lib"]
        for target in EXECUTION_TESTS:
            tests += ["--test", target]
        steps.append(("execution-tests", [*tests, *offline]))
        lint_targets = ["--lib", "--bins"]
    elif not focused:
        steps += [
            ("tests", [cargo, "test", "--workspace", *offline]),
            ("release-tests", [cargo, "test", "--workspace", "--release", *offline]),
            ("trace-tests", [cargo, "test", "--workspace", "--all-features", *offline]),
        ]
        lint_targets = ["--all-targets", "--all-features"]
    if not focused:
        steps.append(
            ("clippy", [cargo, "clippy", "--workspace", *lint_targets, *offline, "--", "-D", "warnings"])
        )
    if guests and not supplied:
        steps.append(
            (f"{profile}-build", [cargo, "build", "--workspace", "--profile", profile, *offline])
        )
    selection = args.case if focused else list(EXECUTION_CASES) if args.execution else []
    cases = [part for case in selection for part in ("--case", case)]
    if not focused:
        steps += [
            ("python-tests", [sys.executable, "-m", "unittest", "discover", "-s", "tools/tests", "-v"]),
            ("suite-tests", [sys.executable, "-m", "unittest", "discover", "-s", str(suite / "tests"), "-v"]),
        ]
    if guests:
        steps.append(("fixture-build", [sys.executable, str(suite / "build.py"), str(out / "fixtures"), *cases]))
    def attempt(name: str, command: list[str], timeout: int = 900) -> None:
        started = time.monotonic()
        try:
            records.append(run(command, out, name, timeout=timeout))
            if name in rust_checks:
                results = re.findall(r"^test result: ok\. (\d+) passed;", (out / f"{name}.log").read_text(), re.MULTILINE)
                if len(results) != rust_checks[name] or any(int(count) == 0 for count in results):
                    raise RuntimeError(f"Rust selection ran no tests or omitted a target; see {out / (name + '.log')}")
        except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
            failure = out / f"{name}-failure.json"
            if not records or records[-1]["name"] != name:
                records.append(
                    json.loads(failure.read_text()) if failure.is_file() else {
                        "name": name, "command": command, "error": str(error),
                        "wall_seconds": time.monotonic() - started,
                    }
                )
            else:
                records[-1]["validation_error"] = str(error)
            raise

    runner = None
    runner_unchanged = None
    failure = None
    try:
        for name, command in steps:
            attempt(name, command)
        if guests and supplied:
            exe = supplied
        elif guests:
            release = release_executable()
            exe = release.parent.parent / profile / release.name
        if guests:
            runner = {"path": str(exe), "sha256": digest(exe), "supplied": supplied is not None}
            attempt(
                "conformance",
                [sys.executable, str(suite / "run.py"),
                 "--adapter", str(ROOT / "tools/hachiware_adapter.py"),
                 "--runner", str(exe), "--fixtures", str(out / "fixtures"),
                 "--out", str(out / "conformance")],
                timeout=240,
            )
            runner_unchanged = runner["sha256"] == digest(exe)
            if not runner_unchanged:
                raise RuntimeError(f"runner changed during execution; see {out / 'summary.json'}")
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
        failure = str(error)
    write_summary(
        out / "summary.json",
        {
            "schema": 1,
            "coverage": "focused" if focused else "execution" if args.execution else "full",
            "rust_profile": profile if focused or args.execution else "dev, release, dev with all features",
            "rust_integration_tests": args.test if focused else list(EXECUTION_TESTS) if args.execution else "all",
            "rust_features": args.features,
            "hachiware_selection": selection if focused or args.execution else "all",
            "runner": runner,
            "runner_unchanged": runner_unchanged,
            "failure": failure,
            "toolchain": versions(),
            "steps": records,
            "hardware_captures": False,
            "private_retail_test": "not run by this command",
        },
        source,
    )
    if failure:
        raise RuntimeError(failure)
    coverage = "Focused checks" if focused else "Execution checks" if args.execution else "Full checks"
    print(f"{coverage} passed. Reports: {out}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError) as error:
        raise SystemExit(str(error))
