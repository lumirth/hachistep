#!/usr/bin/env python3
"""Offline source, Rust, CLI safety and independent hachiware checks."""
from __future__ import annotations
import argparse
import sys
from pathlib import Path
from _support import ROOT, binary, create_directory, run, versions, write_summary, source_identity, release_executable

def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--out', type=Path, default=ROOT / 'out/check')
    p.add_argument('--hachiware', type=Path, default=ROOT.parent / 'hachiware',
                   help='separate hardware suite checkout (default: ../hachiware)')
    a = p.parse_args()
    suite = a.hachiware.resolve()
    if not (suite / 'build.py').is_file() or not (suite / 'run.py').is_file():
        raise RuntimeError('hachiware checkout required: gh repo clone lumirth/hachiware ../hachiware; or pass --hachiware PATH')
    cargo, rustfmt = binary('cargo'), binary('rustfmt')
    source = source_identity()
    out = create_directory(a.out)
    records = []
    sources = sorted(str(f.relative_to(ROOT)) for f in (ROOT / 'crates').rglob('*.rs'))
    steps = [
        ('format', [rustfmt, '--check', '--edition', '2021', *sources]),
        ('tests', [cargo, 'test', '--workspace', '--locked', '--offline']),
        ('release-tests', [cargo, 'test', '--workspace', '--release', '--locked', '--offline']),
        ('trace-tests', [cargo, 'test', '--workspace', '--all-features', '--locked', '--offline']),
        ('clippy', [cargo, 'clippy', '--workspace', '--all-targets', '--all-features', '--locked', '--offline', '--', '-D', 'warnings']),
        ('release-build', [cargo, 'build', '--workspace', '--release', '--locked', '--offline']),
        ('python-tests', [sys.executable, '-m', 'unittest', 'discover', '-s', 'tools/tests', '-v']),
        ('suite-tests', [sys.executable, '-m', 'unittest', 'discover', '-s', str(suite / 'tests'), '-v']),
        ('fixture-build', [sys.executable, str(suite / 'build.py'), str(out / 'fixtures')]),
    ]
    for name, command in steps:
        records.append(run(command, out, name, timeout=900))
    exe = release_executable()
    records.append(run([sys.executable, str(suite / 'run.py'), '--adapter', str(ROOT / 'tools/hachiware_adapter.py'), '--runner', str(exe),
                        '--fixtures', str(out / 'fixtures'), '--out', str(out / 'conformance')], out, 'conformance'))
    write_summary(out / 'summary.json', {'schema': 1, 'toolchain': versions(), 'steps': records,
                                       'hardware_captures': False, 'private_retail_test': 'not run by this command'}, source)
    print(f'Checks passed. Reports: {out}')
if __name__ == '__main__':
    try:
        main()
    except (OSError, RuntimeError, ValueError) as e:
        raise SystemExit(str(e))
