"""Run hachiware experiments through HachiStep and export requested observations."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
from pathlib import Path

from _support import cli_command

STORAGE = {
    "ram": "ram.bin",
    "eeprom": "eeprom.bin",
    "lcd": "lcd-ram.bin",
    "icons": "lcd-icons.bin",
    "pixels": "frame.pgm",
}
SCALARS = (
    "er0",
    "sleeping",
    "interrupt_entries",
    "nv_commits",
    "ir_events",
    "display_on",
    "display_start",
)
# Board assumptions are described in the power, ADC and BMA150 research notes.
CONDITIONS = {
    "reset_resistance_ohms": 100000,
    "reset_capacitance_nf": 100,
    "reset_threshold_vcc_fraction": 0.8,
    "ram_retention_floor_mv": 1500,
    "ram_retention_exposure_mv_ms": 15000,
    "battery_sense_drop_mv": 600,
    "sensor_startup_us": 3000,
    "sensor_scan_order": "temperature,x,y,z",
}


def capabilities() -> dict:
    return {
        "target": "H8/38606F",
        "observations": sorted([*STORAGE, *SCALARS]),
        "inputs": [
            "ir",
            "nmi",
            "reset",
            "power",
            "supply",
            "temperature",
            "accel",
            "buttons",
            *[
                f"digital:{pin}"
                for pin in (
                    "p10",
                    "p11",
                    "p12",
                    "p30",
                    "p31",
                    "p32",
                    "p90",
                    "p91",
                    "p92",
                    "p93",
                    "adtrg",
                )
            ],
            *[
                f"analog:{pin}"
                for pin in ("pb0", "pb1", "pb2", "pb3", "pb4", "pb5", "vcref")
            ],
        ],
        "conditions": CONDITIONS,
    }


def observations(report: dict, milliseconds: int, requested: list[str]) -> dict:
    end = (milliseconds << 64) // 1000
    completed = (
        int(report["time_raw"]) == end
        and int(report["requested_time_raw"]) == end
        and report["time_us"] == (end * 1_000_000 >> 64)
    )
    result = {"fault": report["fault"], "completed": completed}
    for field in requested:
        if field in SCALARS:
            result[field] = report["er"][0] if field == "er0" else report[field]
    return result


def export(output: Path, requested: list[str]) -> None:
    for field in requested:
        if field not in STORAGE:
            continue
        source = output / STORAGE[field]
        destination = output / f"{field}.bin"
        if field == "pixels":
            pgm = source.read_bytes()
            header = b"P5\n96 64\n255\n"
            if not pgm.startswith(header) or len(pgm) != len(header) + 6144:
                raise ValueError("unexpected LCD raster export")
            with destination.open("xb") as stream:
                stream.write(bytes((255 - value) // 85 for value in pgm[len(header) :]))
        elif source != destination:
            with source.open("rb") as incoming, destination.open("xb") as outgoing:
                shutil.copyfileobj(incoming, outgoing)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--describe", action="store_true", help="describe supported experiments"
    )
    parser.add_argument("--runner", type=Path)
    parser.add_argument("--firmware", type=Path)
    parser.add_argument("--eeprom", type=Path)
    parser.add_argument("--milliseconds", type=int)
    parser.add_argument("--input", type=Path)
    parser.add_argument("--out", type=Path)
    parser.add_argument(
        "--observe", action="append", default=[], choices=[*STORAGE, *SCALARS]
    )
    args = parser.parse_args()
    if args.describe:
        print(json.dumps(capabilities(), indent=2))
        return 0
    for field in ("runner", "firmware", "eeprom", "milliseconds", "out"):
        if getattr(args, field) is None:
            parser.error(f"--{field} is required for execution")
    if args.milliseconds <= 0:
        parser.error("--milliseconds must be positive")
    command = [
        "run",
        "--firmware",
        str(args.firmware.resolve()),
        "--eeprom",
        str(args.eeprom.resolve()),
        "--milliseconds",
        str(args.milliseconds),
        "--battery-drop-mv",
        str(CONDITIONS["battery_sense_drop_mv"]),
        "--out",
        str(args.out.resolve()),
    ]
    if args.input:
        command += ["--input", str(args.input.resolve())]
    result = subprocess.run(cli_command(args.runner, command), check=False)
    report_path = args.out / "report.json"
    if report_path.is_file():
        report = json.loads(report_path.read_text())
        export(args.out, args.observe)
        with (args.out / "observations.json").open("x", encoding="utf-8") as stream:
            json.dump(observations(report, args.milliseconds, args.observe), stream)
            stream.write("\n")
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
