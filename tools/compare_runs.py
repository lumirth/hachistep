#!/usr/bin/env python3
"""Compare complete exported observations, optionally including product history.

This is a software-equivalence check, not a hardware oracle. It runs outside the
simulation's timed path. It rejects truncated/missing histories when requested
and reports the first differing byte/record instead of hiding it behind counts.
"""
from __future__ import annotations
import argparse
import itertools
import json
from pathlib import Path
from _support import digest, write_json

REPORT_KEYS = (
    'firmware_sha256', 'initial_eeprom_sha256', 'initial_sensor_nv_sha256',
    'initial_eeprom_status', 'input_sha256', 'initial_conditions',
    'requested_time_raw', 'time_raw', 'er', 'ccr', 'pc', 'instruction_pc', 'phase',
    'retired', 'interrupt_entries', 'sleeping', 'display_on', 'display_start',
    'events', 'lcd_events', 'nv_commits', 'buzzer_events', 'ir_events',
    'serial_tx', 'serial_rx', 'bus_reads', 'bus_writes', 'resets', 'fault',
)
EXPORTS = ('ram.bin', 'eeprom.bin', 'eeprom.status', 'sensor-nv.bin', 'lcd-ram.bin', 'frame.pgm')


def compare(left: Path, right: Path, left_trace: Path | None = None,
            right_trace: Path | None = None) -> dict:
    reports = [json.loads((path/'report.json').read_text()) for path in [left, right]]
    errors = []
    for side, report in zip(['left', 'right'], reports):
        if report['fault'] is not None or report['time_raw'] != report['requested_time_raw']:
            errors.append(f'{side}: fault or incomplete run')
    for key in REPORT_KEYS:
        if reports[0][key] != reports[1][key]:
            errors.append(f'report.{key}: {reports[0][key]!r} != {reports[1][key]!r}')
    hashes = {}
    for name in EXPORTS:
        paths = [side/name for side in [left, right]]
        values = [digest(path) for path in paths]
        hashes[name] = values
        if values[0] != values[1]:
            with paths[0].open('rb') as a, paths[1].open('rb') as b:
                data_a, data_b = a.read(), b.read()
            offset = next(i for i, (a, b) in enumerate(itertools.zip_longest(data_a, data_b)) if a != b)
            errors.append(f'{name}: first difference at byte {offset}')
    history = None
    if left_trace is not None or right_trace is not None:
        if left_trace is None or right_trace is None:
            raise ValueError('both traces are required for history comparison')
        for side, report in zip(['left', 'right'], reports):
            if not report['trace_complete'] or report['trace_dropped']:
                raise ValueError(f'{side}: incomplete or absent trace cannot establish equivalence')
            if report['trace_records'] != report['events']:
                raise ValueError(f'{side}: comparison expects product-only history, not mixed bus traces')
        count_a = count_b = 0
        first = None
        with left_trace.open('rb') as a, right_trace.open('rb') as b:
            for index, (record_a, record_b) in enumerate(itertools.zip_longest(a, b)):
                count_a += record_a is not None
                count_b += record_b is not None
                if first is None and record_a != record_b:
                    first = index
        if [count_a, count_b] != [r['trace_records'] for r in reports]:
            raise ValueError('actual history length does not match its report')
        history = {'records': [count_a, count_b], 'sha256': [digest(left_trace), digest(right_trace)],
                   'first_different_record': first}
        if first is not None:
            errors.append(f'product history: first difference at record {first}')
    return {'schema': 1, 'equivalent': not errors, 'differences': errors,
            'exports_sha256': hashes, 'history': history,
            'scope': 'Exported state and requested product history; not complete hidden state or hardware conformance.'}


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('left', type=Path)
    p.add_argument('right', type=Path)
    p.add_argument('--left-trace', type=Path)
    p.add_argument('--right-trace', type=Path)
    p.add_argument('--report', type=Path)
    args = p.parse_args()
    result = compare(args.left, args.right, args.left_trace, args.right_trace)
    if args.report:
        write_json(args.report, result)
    print(json.dumps(result, indent=2))
    if not result['equivalent']:
        raise SystemExit(1)


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError) as error:
        raise SystemExit(str(error))
