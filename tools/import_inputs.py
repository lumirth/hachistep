#!/usr/bin/env python3
"""Extract only the retail ROM from pw-inputs and copy a separately supplied EEPROM.

Never extracts arbitrary ZIP paths, executes archive content, or writes into an
existing destination. The pw-inputs archive used for this release has no EEPROM.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import zipfile

ROM_MEMBER = 'pw-inputs/firmware/nintendo/pokewalker/retail-48k/pokewalker.bin'
ROM_SIZE, EEPROM_SIZE = 49152, 65536

def import_images(archive: Path, eeprom: Path, destination: Path) -> dict:
    if destination.exists():
        raise FileExistsError(f'refusing existing destination: {destination}')
    if eeprom.stat().st_size != EEPROM_SIZE:
        raise ValueError('EEPROM must be exactly 65536 bytes')
    with zipfile.ZipFile(archive) as z:
        matching = [i for i in z.infolist() if i.filename == ROM_MEMBER]
        if len(matching) != 1 or matching[0].file_size != ROM_SIZE:
            raise ValueError(f'expected one {ROM_SIZE}-byte member: {ROM_MEMBER}')
        firmware = z.read(matching[0])
    eeprom_bytes = eeprom.read_bytes()
    if len(firmware) != ROM_SIZE or len(eeprom_bytes) != EEPROM_SIZE:
        raise ValueError('input size changed while reading')
    manifest = {'schema': 1, 'private_inputs': True, 'rom_member': ROM_MEMBER,
                'firmware_sha256': hashlib.sha256(firmware).hexdigest(),
                'eeprom_sha256': hashlib.sha256(eeprom_bytes).hexdigest(),
                'eeprom_source': 'separate supplied raw EEPROM, not the archive'}
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.mkdir()
    for name, data in [('pokewalker.bin', firmware), ('eeprom.bin', eeprom_bytes)]:
        with (destination / name).open('xb') as f:
            f.write(data)
    with (destination / 'import.json').open('x', encoding='utf-8') as f:
        json.dump(manifest, f, indent=2)
        f.write('\n')
    return manifest

def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--archive', type=Path, required=True)
    p.add_argument('--eeprom', type=Path, required=True)
    p.add_argument('--destination', type=Path, required=True)
    a = p.parse_args()
    print(json.dumps(import_images(a.archive, a.eeprom, a.destination), indent=2))
if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, zipfile.BadZipFile) as e:
        raise SystemExit(str(e))
