#!/usr/bin/env python3
"""Run private-input replay checks and real firmware workloads; never update inputs."""
from __future__ import annotations
import argparse
import json
import sys
from pathlib import Path
from retail_expectations import load as load_expectations, check as check_expectations
from _support import ROOT, binary, create_directory, digest, environment, run, versions, write_json, source_identity, release_executable

def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--firmware', type=Path, default=ROOT / 'local-inputs/pokewalker.bin')
    p.add_argument('--eeprom', type=Path, default=ROOT / 'local-inputs/eeprom.bin')
    p.add_argument('--out', type=Path, default=ROOT / 'out/retail-check')
    p.add_argument('--quick', action='store_true', help='omit walking and long-idle workloads')
    p.add_argument('--menu-trace', action='store_true', help='retain a complete product trace for audio rendering')
    p.add_argument('--expected', type=Path, default=ROOT / 'conformance/regressions/private-retail.json')
    p.add_argument('--smoke-only', action='store_true', help='check execution only; do not claim a behavioral regression match')
    a = p.parse_args()
    firmware, eeprom = a.firmware.resolve(), a.eeprom.resolve()
    before = {'firmware': digest(firmware), 'eeprom': digest(eeprom)}
    expected = None if a.smoke_only else load_expectations(a.expected, before)
    out = create_directory(a.out)
    cargo = binary('cargo')
    env = environment()
    env.update(HS_FIRMWARE=str(firmware), HS_EEPROM=str(eeprom))
    records = []
    records.append(run([cargo, 'test', '-p', 'hs-core', '--release', '--test', 'retail', '--locked', '--offline', '--', '--ignored', '--nocapture'], out, 'partition-and-snapshot', env))
    records.append(run([cargo, 'build', '-p', 'hs-cli', '--release', '--locked', '--offline'], out, 'build'))
    exe = release_executable()
    cases = [('home', 10_000, None), ('menu', 6_500, ROOT / 'conformance/scenarios/menu.csv')]
    if not a.quick:
        cases += [('walking', 61_000, ROOT / 'conformance/scenarios/walking.csv'), ('idle', 120_000, None)]
    summaries = []
    mismatches = []
    for name, ms, timeline in cases:
        command = [str(exe), 'run', '--firmware', str(firmware), '--eeprom', str(eeprom),
                   '--milliseconds', str(ms), '--out', str(out / name)]
        if timeline:
            command += ['--input', str(timeline)]
        if name == 'menu' and a.menu_trace:
            command += ['--trace', str(out / 'menu-events.txt'), '--trace-limit', '2000000']
        records.append(run(command, out, name))
        report = json.loads((out / name / 'report.json').read_text())
        if report['fault'] is not None or report['requested_time_raw'] != report['time_raw']:
            raise RuntimeError(f'{name} did not reach its requested exclusive horizon')
        if report['serial_tx'] == 0 or report['interrupt_entries'] == 0:
            raise RuntimeError(f'{name} did not exercise the expected boot/peripheral integration')
        differences = [] if expected is None else check_expectations(expected['cases'][name], ms, timeline, out / name, report)
        mismatches += [f'{name}: {difference}' for difference in differences]
        summaries.append({'regression_checked': expected is not None, 'regression_differences': differences, 'name': name, 'milliseconds': ms,
                          'frame_sha256': digest(out / name / 'frame.pgm'),
                          'report': report})
    after = {'firmware': digest(firmware), 'eeprom': digest(eeprom)}
    if before != after:
        raise RuntimeError('source input changed')
    write_json(out / 'summary.json', {'schema': 1, 'kind': 'emulator observations, not hardware conformance',
                                     'source': source_identity(), 'toolchain': versions(), 'inputs': before, 'inputs_unchanged': True,
                                     'regression_checked': expected is not None, 'regression_passed': not mismatches if expected else None,
                                     'expectation_file_sha256': digest(a.expected) if expected else None,
                                     'basis_revision': expected['basis_revision'] if expected else None,
                                     'workloads': summaries, 'commands': records})
    if mismatches:
        raise RuntimeError('Regression differences (not automatically emulator bugs):\n' + '\n'.join(mismatches))
    print(f'Retail {"software-regression" if expected else "execution-only smoke"} passed; source images unchanged. Results: {out}')
if __name__ == '__main__':
    try:
        main()
    except (OSError, RuntimeError, ValueError, KeyError) as e:
        raise SystemExit(str(e))
