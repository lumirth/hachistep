#!/usr/bin/env python3
"""Thin HachiStep adapter for core-independent diagnostic images.

Inputs and expectations are validated before any guest is run. A measured result,
software expectation, unresolved question, and runner failure remain distinct.
The adapter never imports CPU semantics or generates expected values.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile

TARGET = 'H8/38606F'
KINDS = {'documented', 'software_reasoned', 'hardware_measured', 'regression', 'unresolved'}
COUNTS = {'nv_commits', 'ir_events', 'interrupt_entries'}
SCALARS = COUNTS | {'er0', 'display_on', 'display_start', 'sleeping'}


def checked_file(root: Path, name: str, expected_hash: str, size: int | None = None) -> Path:
    if not isinstance(name, str) or Path(name).name != name or name in {'', '.', '..'}:
        raise ValueError('fixture members must be simple filenames')
    path = root / name
    if path.resolve().parent != root.resolve():
        raise ValueError('fixture member escapes its directory')
    data = path.read_bytes()
    if size is not None and len(data) != size:
        raise ValueError(f'{name}: expected {size} bytes, got {len(data)}')
    if hashlib.sha256(data).hexdigest() != expected_hash:
        raise ValueError(f'{name}: fixture SHA-256 mismatch')
    return path


def validate_expected(expected: dict) -> None:
    if not isinstance(expected, dict) or expected.keys() - (SCALARS | {'ram', 'eeprom'}):
        raise ValueError('unknown or malformed expected-result key')
    for domain, start, length in [('ram', 0xf780, 2048), ('eeprom', 0, 65536)]:
        entries = expected.get(domain, {})
        if not isinstance(entries, dict):
            raise ValueError(f'{domain}: expectations must map addresses to hex bytes')
        for address, text in entries.items():
            if not isinstance(address, str) or re.fullmatch('[0-9a-fA-F]{4}', address) is None:
                raise ValueError(f'{domain}: invalid address')
            if not isinstance(text, str) or re.fullmatch('(?:[0-9a-fA-F]{2})+', text) is None:
                raise ValueError(f'{domain}: expected nonempty hex bytes')
            offset = int(address, 16) - start
            if not 0 <= offset <= length - len(bytes.fromhex(text)):
                raise ValueError(f'{domain}: expected range outside physical storage')
    for key in expected.keys() & SCALARS:
        value = expected[key]
        if key in {'display_on', 'sleeping'}:
            if type(value) is not bool:
                raise ValueError(f'{key}: expected Boolean')
        elif type(value) is not int or value < 0:
            raise ValueError(f'{key}: expected nonnegative integer')
        if key == 'er0' and value > 0xffffffff:
            raise ValueError('ER0 does not fit 32 bits')
        if key == 'display_start' and value > 127:
            raise ValueError('display start does not fit seven bits')


def load_manifest(fixtures: Path) -> dict:
    manifest = json.loads((fixtures / 'manifest.json').read_text())
    if manifest.get('schema') != 2:
        raise ValueError('expected fixture schema 2; rebuild fixtures with build.py')
    if not isinstance(manifest.get('target'), str) or not manifest['target']:
        raise ValueError('missing fixture target')
    eeprom = manifest['eeprom']
    checked_file(fixtures, eeprom['file'], eeprom['sha256'], 65536)
    cases = manifest['cases']
    if not isinstance(cases, list) or not cases:
        raise ValueError('empty or malformed fixture corpus')
    names = set()
    for case in cases:
        name = case['name']
        if not isinstance(name, str) or re.fullmatch('[a-z0-9-]+', name) is None or name in names:
            raise ValueError('duplicate or unsafe fixture identifier')
        names.add(name)
        checked_file(fixtures, case['firmware'], case['sha256'], 49152)
        if case.get('input') is not None:
            checked_file(fixtures, case['input'], case['input_sha256'])
        elif case.get('input_sha256') is not None:
            raise ValueError('timeline hash supplied without a timeline')
        milliseconds = case['milliseconds']
        if type(milliseconds) is not int or not 1 <= milliseconds <= 120000:
            raise ValueError('diagnostic duration must be 1..120000 ms')
        basis = case['expectation']
        if basis.get('kind') not in KINDS or not basis.get('question') or not basis.get('source'):
            raise ValueError('missing expectation provenance/question')
        if basis['kind'] == 'hardware_measured' and not basis.get('observation_id'):
            raise ValueError('a hardware expectation must identify its captured observation')
        validate_expected(case['expected'])
        if not case['expected'] and basis['kind'] != 'unresolved':
            raise ValueError('an established test must make an assertion')
    return manifest


def compare(expected: dict, report: dict, output: Path, milliseconds: int) -> list[str]:
    failures = []
    if report['fault'] is not None:
        failures.append(f'model fault: {report["fault"]}')
    expected_raw = (milliseconds << 64) // 1000
    if (int(report['time_raw']) != expected_raw or int(report['requested_time_raw']) != expected_raw
            or report['time_us'] != (expected_raw * 1_000_000 >> 64)):
        failures.append('requested exclusive horizon was not reached')
    for domain, start, size in [('ram', 0xf780, 2048), ('eeprom', 0, 65536)]:
        data = (output / f'{domain}.bin').read_bytes()
        if len(data) != size:
            raise ValueError(f'runner exported wrong {domain} size')
        for address, text in expected.get(domain, {}).items():
            offset = int(address, 16) - start
            wanted = bytes.fromhex(text)
            actual = data[offset:offset+len(wanted)]
            if actual != wanted:
                failures.append(f'{domain}[{address}]: expected {wanted.hex()}, got {actual.hex()}')
    for key in expected.keys() & SCALARS:
        actual = report['er'][0] if key == 'er0' else report[key]
        if actual != expected[key]:
            failures.append(f'{key}: expected {expected[key]}, got {actual}')
    return failures


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runner', type=Path, required=True)
    parser.add_argument('--fixtures', type=Path, required=True)
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    runner, fixtures = args.runner.resolve(), args.fixtures.resolve()
    manifest = load_manifest(fixtures)
    if args.report and args.report.exists():
        raise FileExistsError(args.report)
    results = []
    with tempfile.TemporaryDirectory(prefix='hachistep-conformance-') as directory:
        for case in manifest['cases']:
            failures = []
            status = 'not_applicable' if manifest['target'] != TARGET else 'pass'
            if case['expectation']['kind'] == 'unresolved':
                status = 'unknown'
            if status == 'pass':
                output = Path(directory) / case['name']
                command = [str(runner), 'run', '--firmware', str(fixtures / case['firmware']),
                           '--eeprom', str(fixtures / manifest['eeprom']['file']),
                           '--milliseconds', str(case['milliseconds']), '--out', str(output)]
                if case.get('input'):
                    command += ['--input', str(fixtures / case['input'])]
                try:
                    process = subprocess.run(command, text=True, capture_output=True, timeout=30)
                    if process.returncode:
                        if (output / 'report.json').exists():
                            status = 'fail' # includes explicit unsupported guest behavior
                        else:
                            status = 'runner_error'
                        failures.append(process.stdout + process.stderr)
                    else:
                        report = json.loads((output / 'report.json').read_text())
                        failures = compare(case['expected'], report, output, case['milliseconds'])
                        if failures:
                            status = 'fail'
                except (OSError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
                    status, failures = 'runner_error', [str(error)]
            result = {'case': case['name'], 'status': status, 'passed': status == 'pass',
                      'expectation': case['expectation'], 'failures': failures}
            results.append(result)
            print(status.upper() + ' ' + case['name'])
            for failure in failures:
                print(failure)
    counts = {status: sum(r['status'] == status for r in results)
              for status in ['pass', 'fail', 'unknown', 'not_applicable', 'runner_error']}
    summary = {'schema': 2, 'kind': 'adapter results; provenance is recorded per case',
               'fixture_manifest_sha256': hashlib.sha256((fixtures/'manifest.json').read_bytes()).hexdigest(),
               'target': TARGET, 'counts': counts, 'results': results}
    if args.report:
        with args.report.open('x') as file:
            json.dump(summary, file, indent=2)
            file.write('\n')
    if counts['fail'] or counts['runner_error']:
        raise SystemExit(1)
    if counts['unknown'] or counts['not_applicable']:
        raise SystemExit(2) # unresolved/non-applicable is never silently green


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise SystemExit(str(error))
