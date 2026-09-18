"""Software-observed replay expectations, deliberately not a hardware oracle."""
from __future__ import annotations
import json
from pathlib import Path
from _support import digest
from compare_runs import OBSERVATION_FILES, REPORT_KEYS


def load(path: Path, inputs: dict) -> dict:
    data = json.loads(path.read_text())
    if data.get('schema') != 1 or data.get('kind') != 'software-regression':
        raise ValueError('retail expectations must explicitly identify a software-regression baseline')
    if data.get('hardware_measured') is not False or not data.get('basis_revision'):
        raise ValueError('baseline basis/revision is absent or incorrectly claims physical measurement')
    if data['inputs'] != inputs:
        raise ValueError('private inputs do not match the selected regression baseline; use --smoke-only for different inputs')
    if not data.get('cases'):
        raise ValueError('empty retail expectation set')
    for name, case in data['cases'].items():
        if set(case['report']) != set(REPORT_KEYS):
            raise ValueError(f'{name}: missing or unknown semantic report field')
        # A reviewed behavioral baseline does not freeze the private native
        # encoding. Cross-run equivalence separately compares complete states.
        if set(case['exports']) != set(OBSERVATION_FILES):
            raise ValueError(f'{name}: missing or unknown exported file')
        if type(case['milliseconds']) is not int or case['milliseconds'] < 1:
            raise ValueError(f'{name}: invalid run duration')
        for value in case['exports'].values():
            if len(value) != 64 or any(c not in '0123456789abcdef' for c in value):
                raise ValueError(f'{name}: invalid export SHA-256')
    return data


def check(case: dict, milliseconds: int, timeline: Path | None, directory: Path, report: dict) -> list[str]:
    differences = []
    actual_timeline = digest(timeline) if timeline else None
    if milliseconds != case['milliseconds']:
        differences.append('duration does not match the regression case')
    if actual_timeline != case['input_sha256']:
        differences.append('timeline does not match the regression case')
    for key, expected in case['report'].items():
        if report.get(key) != expected:
            differences.append(f'report.{key}: {report.get(key)!r} != {expected!r}')
    for name, expected in case['exports'].items():
        if digest(directory/name) != expected:
            differences.append(f'{name}: SHA-256 differs from the reviewed software baseline')
    return differences
