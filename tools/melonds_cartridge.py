#!/usr/bin/env python3
"""Rebuild the cartridge adapter with Clang."""
from pathlib import Path
import struct
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1] / 'desktop/crates/hs-runtime/src/melonds'
with tempfile.TemporaryDirectory() as tmp:
    obj = Path(tmp) / 'cartridge.o'
    subprocess.run(['clang', '--target=arm-none-eabi', '-c', str(root / 'cartridge.s'), '-o', str(obj)], check=True)
    data = obj.read_bytes()
    offset = struct.unpack_from('<I', data, 32)[0]
    size, count, names = struct.unpack_from('<3H', data, 46)
    sections = [struct.unpack_from('<10I', data, offset + i * size) for i in range(count)]
    strings = sections[names]
    strings = data[strings[4]:strings[4] + strings[5]]
    for section in sections:
        assert section[1] not in (4, 9), 'The adapter must have no relocations'
        if strings[section[0]:].split(b'\0')[0] == b'.text':
            code = data[section[4]:section[4] + section[5]]
            assert len(code) == 948
            (root / 'cartridge.bin').write_bytes(code)
