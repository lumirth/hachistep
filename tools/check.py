#!/usr/bin/env python3
"""Offline source, Rust, CLI safety and independent-fixture checks."""
from __future__ import annotations
import argparse
import sys
from pathlib import Path
from _support import ROOT, binary, create_directory, run, versions, write_json, source_identity, release_executable

def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--out', type=Path, default=ROOT / 'out/check')
    a = p.parse_args()
    cargo, rustfmt = binary('cargo'), binary('rustfmt')
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
        ('fixture-build', [sys.executable, 'conformance/build.py', str(out / 'fixtures')]),
    ]
    for name, command in steps:
        records.append(run(command, out, name))
    exe = release_executable()
    records.append(run([sys.executable, 'conformance/run.py', '--runner', str(exe),
                        '--fixtures', str(out / 'fixtures'), '--report', str(out / 'conformance.json')], out, 'conformance'))
    write_json(out / 'summary.json', {'schema': 1, 'source': source_identity(), 'toolchain': versions(), 'steps': records,
                                    'hardware_captures': False, 'private_retail_test': 'not run by this command'})
    print(f'Checks passed. Reports: {out}')
if __name__ == '__main__':
    try:
        main()
    except (OSError, RuntimeError, ValueError) as e:
        raise SystemExit(str(e))
