#!/usr/bin/env python3
"""Build small, original H8 diagnostic images without a cross compiler.

This is a fixture encoder, not an assembler or reference emulator. Expectations
are literal, independently reasoned values. No production Rust is imported.
"""
from __future__ import annotations
import argparse, hashlib, json
from pathlib import Path

class Program:
    def __init__(self) -> None:
        self.code = bytearray(bytes.fromhex('7907ff80'))
    def byte(self, address: int, value: int) -> None:
        self.code += bytes((0xF8, value, 0x6A, 0x88, address >> 8, address & 255))
    def send(self, value: int) -> None:
        self.byte(0xF0EB, value)
        self.code += bytes.fromhex('6a08f0e4e80847f86a08f0e9')
    def finish(self) -> bytes:
        self.code += bytes.fromhex('40fe')
        image = bytearray(49152)
        image[:2] = bytes.fromhex('0100')
        image[0x100:0x100 + len(self.code)] = self.code
        return bytes(image)

def cases():
    p = Program()
    p.code += bytes.fromhex('7a0011223344f0aaf8bb7908ccdd01006b80f800')
    yield 'register-aliases', p.finish(), {'ram': {'f800': 'ccddaabb'}}, None
    p = Program()
    p.code += bytes.fromhex('f880888002096a88f8006a89f801')
    yield 'add-byte-flags', p.finish(), {'ram': {'f800': '0087'}}, None
    p = Program()
    p.code += bytes.fromhex('f82a5e0001206a88f80040fe')
    p.code += bytes(0x20-len(p.code))
    p.code += bytes.fromhex('88015470')
    yield 'call-return-stack', p.finish(), {'ram': {'f800': '2b', 'ff7e': '010a'}}, None
    p = Program()
    p.code += bytes.fromhex('7a00f8a540fe01006b80f8205a00f820')
    yield 'execute-from-ram', p.finish(), {'er0': 0xF8A540A5}, None
    p = Program()
    for a,v in [(0xfffb,0x14),(0xf0e0,0x8c),(0xf0e1,0x40),(0xf0e2,0x86),
                (0xf0e3,0xc0),(0xffe4,7),(0xffd4,5),(0xf087,8),(0xffec,1),(0xffdc,1)]:
        p.byte(a,v)
    p.byte(0xffd4,1); p.send(6); p.byte(0xffd4,5)
    p.byte(0xffd4,1)
    for value in [2,0,0x7e,0xaa,0xbb,0xcc,0xdd]: p.send(value)
    p.byte(0xffd4,5)
    yield 'serial-eeprom-page-wrap', p.finish(), {'eeprom': {'007e':'aabb','0000':'ccdd'}, 'nv_commits':1}, None
    p = Program()
    for a,v in [(0xfffa,0x43),(0xff91,0xd0),(0xff99,1),(0xffa7,0x80),(0xff9a,0x20),(0xff9b,0xa5)]:
        p.byte(a,v)
    yield 'infrared-transmit', p.finish(), {'ir_events':10}, None
    p = Program()
    for a,v in [(0xfffa,0x43),(0xff91,0xd0),(0xff99,1),(0xffa7,0x80),(0xff9a,0x10)]: p.byte(a,v)
    p.code += bytes.fromhex('6a08ff9ce84047f86a08ff9d6a88f800')
    bits = [0] + [(0xa5 >> i) & 1 for i in range(8)] + [1]
    rows = ['# Nominal SIR pulses; digital software fixture, not a hardware capture.']
    for i,one in enumerate(bits):
        if not one:
            at = 500 + round(i*64*1_000_000/3_686_400)
            rows.extend([f'{at},ir,1',f'{at+3},ir,0'])
    yield 'infrared-receive', p.finish(), {'ram':{'f800':'a5'}}, '\n'.join(rows)+'\n'

def main() -> None:
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('output',type=Path)
    args=ap.parse_args(); args.output.mkdir(parents=True,exist_ok=False)
    (args.output/'blank-eeprom.bin').write_bytes(bytes([255])*65536)
    manifest={'schema':1,'basis':'original software fixtures; no physical-hardware certification','cases':[]}
    for name,image,expected,timeline in cases():
        (args.output/f'{name}.bin').write_bytes(image)
        if timeline: (args.output/f'{name}.csv').write_text(timeline)
        manifest['cases'].append({'name':name,'firmware':f'{name}.bin','sha256':hashlib.sha256(image).hexdigest(),
                                  'milliseconds':8,'expected':expected,'input':f'{name}.csv' if timeline else None})
    (args.output/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    print(f'Built {len(manifest["cases"])} independent diagnostic images in {args.output}')
if __name__=='__main__': main()
