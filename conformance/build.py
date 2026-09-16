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
    def word(self, address: int, value: int) -> None:
        self.code += bytes((0x79, 0, value >> 8, value & 255, 0x6B, 0x80, address >> 8, address & 255))
    def send(self, value: int) -> None:
        self.byte(0xF0EB, value)
        self.code += bytes.fromhex('6a08f0e4e80847f86a08f0e9')
    def finish(self) -> bytes:
        self.code += bytes.fromhex('40fe')
        image = bytearray(49152)
        image[:2] = bytes.fromhex('0100')
        if len(self.code) > 49152 - 0x100:
            raise ValueError('diagnostic code exceeds target flash')
        image[0x100:0x100 + len(self.code)] = self.code
        return bytes(image)

def handler(image: bytes, vector: int, code: str) -> bytes:
    image = bytearray(image)
    body = bytes.fromhex(code)
    if any(image[0x200:0x200+len(body)]):
        raise ValueError('handler overlaps diagnostic program')
    image[vector*2:vector*2+2] = bytes.fromhex('0200')
    image[0x200:0x200+len(body)] = body
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

    # MOV stores must sample an aliased register *after* predecrement.
    # ADE-602-053A pp.121,123,125. Literal expectations are not obtained
    # by executing HachiStep or importing its decoder.
    for suffix, opcode, address, expected in [
        ('byte', '6ca2', 'f7ff', 'f7'),
        ('word', '6da2', 'f7fe', 'f7fe'),
        ('long', '01006da2', 'f7fc', '1122f7fc'),
    ]:
        p=Program();p.code += bytes.fromhex('7a021122f800' + opcode)
        yield 'predecrement-alias-'+suffix, p.finish(), {'ram':{address:expected}}, None

    p=Program()
    p.byte(0xfffb,0x44);p.byte(0xffe4,7);p.byte(0xffd4,4)
    p.word(0xf0f6,0x1234);p.byte(0xf0f4,0x8c);p.byte(0xf0f2,1);p.byte(0xffd4,5)
    p.code += bytes.fromhex('067f018040fc')
    image=handler(p.finish(),35,'6b00f0f86b80f7806a08f0f3f8006a88f0f35670')
    yield 'timer-w-gpio-capture',image,{'ram':{'f780':'1234'},'interrupt_entries':1},None

    p=Program();p.byte(0xfffb,6);p.byte(0xf0dc,0xc8)
    p.code += bytes(128) # settle before arming the comparator through CMDR
    p.code += bytes.fromhex('6a08f0de067f018040fc')
    image=handler(p.finish(),36,'f8016a88f7806a08f0def8006a88f0de5670')
    timeline='0,analog,pb4,1000\n200,analog,pb4,2100\n'
    yield 'comparator-read-armed-wake',image,{'ram':{'f780':'01'},'interrupt_entries':1},timeline

    for gate_irq in [False,True]:
        p=Program()
        for a,v in [(0xfffb,12),(0xffc0,8),(0xffc0,0x28),(0xff92,0x10),(0xff95,0x17),
                    (0xfff3 if gate_irq else 0xfff4,4 if gate_irq else 1)]:p.byte(a,v)
        p.code += bytes.fromhex('067f018040fc')
        isr='f8016a88f7802895f8173895f80038'+('f6' if gate_irq else 'f7')+'5670'
        image=handler(p.finish(),18 if gate_irq else 32,isr)
        rows=['0,digital,p11,0','0,digital,p12,1']
        if gate_irq:rows += ['100,digital,p12,0']
        else:
            for i in range(256):rows += [f'{100+10*i},digital,p11,1',f'{105+10*i},digital,p11,0']
        yield 'aec-gate-interrupt' if gate_irq else 'aec-external-overflow',image,{
            'ram':{'f780':'01'},'interrupt_entries':1},'\n'.join(rows)+'\n'

    p=Program();p.code += bytes.fromhex('0780018040fc')
    image=handler(p.finish(),7,'6a08f7800a086a88f7805670')
    yield 'nmi-masked-sleep',image,{'ram':{'f780':'02'},'interrupt_entries':2},'100,nmi,0\n300,nmi,1\n500,nmi,0\n'


def expectation_metadata(name: str) -> dict:
    if name.startswith('predecrement'):
        return {'kind':'documented','source':'ADE-602-053A MOV.B/W/L usage notes pp.121/123/125',
                'question':'Does predecrement precede sampling an aliased source field?'}
    if name.startswith('aec'):
        return {'kind':'documented','source':'REJ09B0152-0300 §§13.3–13.5',
                'question':'Do actual package inputs produce the specified AEC count/gate interrupt?'}
    if name.startswith('timer-w'):
        return {'kind':'documented','source':'REJ09B0152-0300 §§10.4.3,10.6',
                'question':'Does a rising capture record a stopped counter and request vector 35?'}
    if name.startswith('comparator'):
        return {'kind':'documented','source':'REJ09B0152-0300 §§18.3–18.4',
                'question':'Does CMDR read arm comparison and permit a settled difference to wake via vector 36?',
                'limitation':'Large voltage/time margins avoid relying on provisional analog response delay.'}
    if name.startswith('nmi'):
        return {'kind':'documented','source':'REJ09B0152-0300 §§3.4.1,3.5.1,5.2',
                'question':'Does each selected NMI edge wake despite I=1, without retriggering a held level?'}
    return {'kind':'software_reasoned','source':'Original starter fixture; see conformance/README.md',
            'question':'Check the named CPU/protocol mechanism against its literal expected result.'}


def main() -> None:
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('output',type=Path)
    args=ap.parse_args(); args.output.mkdir(parents=True,exist_ok=False)
    (args.output/'blank-eeprom.bin').write_bytes(bytes([255])*65536)
    manifest={'schema':2,'target':'H8/38606F','basis':'Documented and software-reasoned expectations; no physical captures',
              'eeprom':{'file':'blank-eeprom.bin','sha256':hashlib.sha256(bytes([255])*65536).hexdigest()},'cases':[]}
    for name,image,expected,timeline in cases():
        (args.output/f'{name}.bin').write_bytes(image)
        if timeline: (args.output/f'{name}.csv').write_text(timeline)
        manifest['cases'].append({'name':name,'firmware':f'{name}.bin','sha256':hashlib.sha256(image).hexdigest(),
                                  'milliseconds':8,'expected':expected,'input':f'{name}.csv' if timeline else None,
                                  'input_sha256':hashlib.sha256(timeline.encode()).hexdigest() if timeline else None,
                                  'expectation':expectation_metadata(name)})
    (args.output/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    print(f'Built {len(manifest["cases"])} independent diagnostic images in {args.output}')
if __name__=='__main__': main()
