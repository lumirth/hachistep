#!/usr/bin/env python3
"""Confirm selected tests reject deliberate hardware defects in isolated copies.

This is a test-sensitivity check, not hardware certification. The source checkout
is never modified. Mutants must compile and fail a named assertion test; compiler
errors, missing tests and unapplied mutations are failures of this tool.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
from _support import ROOT, binary, create_directory, environment, source_identity, versions, write_json

ADDRESS = '                let (mut address, post) = self.target_address(address, size);\n'
VALUE = '''                let value = if ccr {
                    u32::from(self.registers.ccr) << 8
                } else {
                    self.registers.read(size, reg)
                };
'''
CASES = (
    ('rtc-extra-access-state', 'crates/hs-core/src/mcu/mod.rs',
     'if matches!(a, 0xf0e0..=0xf0e4 |', 'if matches!(a, 0xf068 | 0xf0e0..=0xf0e4 |',
     'register_access', 'complete_documented_register_width_and_timing_map'),
    ('predecrement-stale-source', 'crates/hs-core/src/cpu/mod.rs',
     ADDRESS + VALUE, VALUE + ADDRESS,
     'cpu_regressions', 'predecrement_reads_updated_aliased_source_at_every_width_and_register'),
    ('eepmov-word-defers-nmi', 'crates/hs-core/src/cpu/mod.rs',
     'if word_count && stage == 2 && interrupt == Some(7) {',
     'if false && word_count && stage == 2 && interrupt == Some(7) {',
     'cpu_regressions', 'eepmov_word_accepts_nmi_only_between_complete_byte_transfers'),
)


def execute(cargo: str, tree: Path, target: Path, test: str, name: str, log: Path) -> tuple[int, str]:
    env = environment()
    env['CARGO_TARGET_DIR'] = str(target)
    env['CARGO_INCREMENTAL'] = '0'
    result = subprocess.run([cargo, 'test', '-p', 'hs-core', '--locked', '--offline',
                             '--test', test, name, '--', '--exact', '--nocapture'],
                            cwd=tree, env=env, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, timeout=180)
    log.write_text(result.stdout)
    return result.returncode, result.stdout


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--out', type=Path, default=ROOT/'out/mutations')
    args = p.parse_args()
    out = create_directory(args.out)
    cargo = binary('cargo')
    records = []
    with tempfile.TemporaryDirectory(prefix='hachistep-mutations-') as directory:
        temp = Path(directory)
        tree = temp/'source'
        tree.mkdir()
        for name in ('Cargo.toml', 'Cargo.lock'):
            shutil.copy2(ROOT/name, tree/name)
        shutil.copytree(ROOT/'crates', tree/'crates', ignore=shutil.ignore_patterns('target', '__pycache__'))
        shutil.copytree(ROOT/'conformance/spec', tree/'conformance/spec')
        for ident, path, before, after, test, name in CASES:
            file = tree/path
            original = file.read_text()
            if original.count(before) != 1:
                raise ValueError(f'{ident}: expected exactly one mutation site')
            code, text = execute(cargo, tree, temp/'target', test, name, out/f'{ident}-control.log')
            if code != 0 or '1 passed; 0 failed;' not in text or f'test {name} ... ok' not in text:
                raise RuntimeError(f'{ident}: unmodified control did not pass exactly the expected test')
            try:
                file.write_text(original.replace(before, after, 1))
                code, text = execute(cargo, tree, temp/'target', test, name, out/f'{ident}-mutant.log')
                rejected = (code == 101 and '0 passed; 1 failed;' in text
                            and f'test {name} ... FAILED' in text
                            and 'panicked at' in text and 'could not compile' not in text)
                records.append({'id': ident, 'test': f'{test}::{name}', 'control_passed': True,
                                'mutant_compiled_and_rejected': rejected, 'returncode': code})
                if not rejected:
                    raise RuntimeError(f'{ident}: mutation was not rejected by the expected assertion')
                print(f'{ident}: control passes; compiled mutant fails the named test', flush=True)
            finally:
                file.write_text(original)
    write_json(out/'summary.json', {'schema': 1, 'source': source_identity(), 'toolchain': versions(),
                                   'kind': 'test sensitivity, not hardware observations', 'profile': 'debug with overflow checks', 'cases': records})

if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        raise SystemExit(str(error))
