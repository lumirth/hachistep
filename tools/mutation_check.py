#!/usr/bin/env python3
"""Confirm selected tests reject deliberate hardware defects in isolated copies.

Mutants must compile and fail a named assertion test. Compiler errors, missing
tests and unapplied mutations are failures of this tool.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
from _support import ROOT, binary, create_directory, environment, source_identity, versions, write_summary

ADDRESS = '                let (mut address, post) = self.target_address(address, size);\n'
VALUE = '''                let value = if ccr {
                    u32::from(self.registers.ccr) << 8
                } else {
                    self.registers.read(size, reg)
                };
'''
CASES = (
    ('rtc-extra-access-state', 'crates/hs-core/src/mcu/bus.rs',
     'if matches!(a, 0xf0e0..=0xf0e4 |', 'if matches!(a, 0xf068 | 0xf0e0..=0xf0e4 |',
     'register_access', 'actual_guest_accesses_commit_at_the_documented_exclusive_boundary'),
    ('predecrement-stale-source', 'crates/hs-core/src/cpu/mod.rs',
     ADDRESS + VALUE, VALUE + ADDRESS,
     'cpu_regressions', 'predecrement_reads_updated_aliased_source_at_every_width_and_register'),
    ('eepmov-word-defers-nmi', 'crates/hs-core/src/cpu/mod.rs',
     'if word_count && interrupt() == Some(7) {',
     'if false && word_count && interrupt() == Some(7) {',
     'cpu_regressions', 'eepmov_word_accepts_nmi_only_between_complete_byte_transfers'),
    ('sci-read-retains-interrupt', 'crates/hs-core/src/mcu/bus.rs',
     '0xf0e9 | 0xf07f | 0xff9d | 0xf0de', '0xf0e9 | 0xf07f | 0xf0de',
     'interrupt_reads', 'reading_sci_data_while_masked_removes_the_request_before_unmasking'),
    ('coincident-serial-edge-loses-driver', 'crates/hs-core/src/machine/serial.rs',
     'owners: owners | schedule::SSU,', 'owners,',
     'serial_appointments', 'serial_bytes_survive_unrelated_device_appointments'),
    ('ram-trap-protects-flash-at-stale-time', 'crates/hs-core/src/machine/execution.rs',
     '                    let (action, _) = self.apply_cpu_request(request, out)?;',
     '''                    let completed_at = self.now;
                    self.now = pending.wait.deadline(&self.mcu.clocks)?.unwrap();
                    let result = self.apply_cpu_request(request, out);
                    self.now = completed_at;
                    let (action, _) = result?;''',
     'flash_execution', 'a_ram_trap_protects_flash_at_exception_admission_after_the_completed_pulse_prefix'),
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
    source = source_identity()
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
    write_summary(out/'summary.json', {'schema': 1, 'toolchain': versions(),
                                      'kind': 'test sensitivity, not hardware observations', 'profile': 'debug with overflow checks', 'cases': records}, source)

if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        raise SystemExit(str(error))
