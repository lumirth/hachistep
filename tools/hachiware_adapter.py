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
    "lcd_supply": "lcd-supply.bin",
    "lcd_drive": "lcd-drive.bin",
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
# Nominal board parameters, documented in docs/hardware.md.
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
        "sessions": ["capture", "history", "chunk", "edit_ram", "audio", "link"],
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
                stream.write(bytes(255 - value for value in pgm[len(header) :]))
        elif source != destination:
            with source.open("rb") as incoming, destination.open("xb") as outgoing:
                shutil.copyfileobj(incoming, outgoing)


def session_export(output: Path, report: dict) -> None:
    fields = {key: report[key] for key in ("sleeping", "display_on", "display_start", "display_contrast")}
    fields["er0"] = report["er"][0]
    result = {
        "fault": report["fault"],
        "start_raw": report["start_time_raw"],
        "end_raw": report["time_raw"],
        "requested_raw": report["requested_time_raw"],
        "observations": fields,
        "artifacts": {**{field: f"{field}.bin" for field in STORAGE}, "state": "state.bin"},
        "counters": {key: report[key] for key in ("interrupt_entries", "nv_commits", "ir_events", "buzzer_events")},
        "history": {"path": "events.txt", "complete": report["trace_complete"],
                    "records": report["trace_records"]},
        "run_seconds": report["wall_seconds"],
    }
    with (output / "session.json").open("x") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")


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
    parser.add_argument("--session", action="store_true")
    parser.add_argument("--load-state", type=Path)
    parser.add_argument("--chunk-us", type=int)
    parser.add_argument("--write-ram", action="append", default=[])
    parser.add_argument("--history", action="store_true")
    parser.add_argument("--audio", action="store_true")
    parser.add_argument("--peer-eeprom", type=Path)
    parser.add_argument("--peer-input", type=Path)
    parser.add_argument("--delay-us", type=int, default=1)
    parser.add_argument(
        "--observe", action="append", default=[], choices=[*STORAGE, *SCALARS]
    )
    args = parser.parse_args()
    if args.describe:
        print(json.dumps(capabilities(), indent=2))
        return 0
    for field in ("runner", "milliseconds", "out"):
        if getattr(args, field) is None:
            parser.error(f"--{field} is required for execution")
    if args.milliseconds < 0 or (not args.session and args.milliseconds == 0):
        parser.error("--milliseconds must be positive, or zero for session inspection")
    if args.load_state:
        initial = ["--load-state", str(args.load_state.resolve())]
    elif args.firmware and args.eeprom:
        initial = ["--firmware", str(args.firmware.resolve()), "--eeprom", str(args.eeprom.resolve()),
                   "--battery-drop-mv", str(CONDITIONS["battery_sense_drop_mv"])]
    else:
        parser.error("provide --firmware and --eeprom, or --load-state")
    command = ["link" if args.peer_eeprom else "run", *initial,
               "--milliseconds", str(args.milliseconds), "--out", str(args.out.resolve())]
    if args.input:
        command += ["--input", str(args.input.resolve())]
    if args.chunk_us:
        command += ["--chunk-us", str(args.chunk_us)]
    for edit in args.write_ram:
        command += ["--write-ram", edit]
    if args.history:
        command += ["--trace", str(args.out.resolve() / "events.txt"), "--trace-limit", "2000000"]
    if args.audio:
        command += ["--audio", str(args.out.resolve() / "audio.wav")]
    if args.peer_eeprom:
        command += ["--peer-eeprom", str(args.peer_eeprom.resolve()), "--delay-us", str(args.delay_us)]
        if args.peer_input:
            command += ["--peer-input", str(args.peer_input.resolve())]
    result = subprocess.run(cli_command(args.runner, command), check=False)
    report_path = args.out / "report.json"
    if report_path.is_file():
        report = json.loads(report_path.read_text())
        requested = sorted(STORAGE) if args.session else args.observe
        export(args.out, requested)
        if args.session:
            session_export(args.out, report)
            if args.peer_eeprom:
                peer = args.out / "peer"
                peer_report = json.loads((peer / "report.json").read_text())
                export(peer, requested)
                session_export(peer, peer_report)
        with (args.out / "observations.json").open("x", encoding="utf-8") as stream:
            json.dump(observations(report, args.milliseconds, args.observe), stream)
            stream.write("\n")
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
