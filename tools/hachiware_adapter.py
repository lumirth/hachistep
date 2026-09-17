#!/usr/bin/env python3
"""Translate HachiStep CLI observations to the hachiware hardware contract."""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import shutil
import subprocess


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runner', type=Path, required=True)
    parser.add_argument('--firmware', type=Path, required=True)
    parser.add_argument('--eeprom', type=Path, required=True)
    parser.add_argument('--milliseconds', type=int, required=True)
    parser.add_argument('--input', type=Path)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    command = [str(args.runner), 'run', '--firmware', str(args.firmware),
               '--eeprom', str(args.eeprom), '--milliseconds', str(args.milliseconds),
               '--out', str(args.out)]
    if args.input:
        command += ['--input', str(args.input)]
    result = subprocess.run(command, check=False)
    report_path = args.out / 'report.json'
    if report_path.is_file():
        shutil.copyfile(args.out / 'lcd-ram.bin', args.out / 'lcd.bin')
        shutil.copyfile(args.out / 'lcd-icons.bin', args.out / 'icons.bin')
        pgm = (args.out / 'frame.pgm').read_bytes()
        header = b'P5\n96 64\n255\n'
        if not pgm.startswith(header) or len(pgm) != len(header) + 6144:
            raise ValueError('unexpected LCD raster export')
        (args.out / 'pixels.bin').write_bytes(bytes((255 - v) // 85 for v in pgm[len(header):]))
        report = json.loads(report_path.read_text())
        fields = ('fault', 'time_raw', 'requested_time_raw', 'time_us', 'er',
                  'sleeping', 'interrupt_entries', 'nv_commits', 'ir_events',
                  'display_on', 'display_start')
        with (args.out / 'observations.json').open('x') as file:
            json.dump({key: report[key] for key in fields}, file)
            file.write('\n')
    return result.returncode


if __name__ == '__main__':
    raise SystemExit(main())
