#!/usr/bin/env python3
"""Adapter for the hachistep CLI. Core-independent fixtures live in build.py."""
from __future__ import annotations
import argparse, hashlib, json, subprocess, tempfile
from pathlib import Path

def main()->None:
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--runner',type=Path,required=True)
    p.add_argument('--fixtures',type=Path,required=True)
    p.add_argument('--report',type=Path)
    a=p.parse_args(); runner=a.runner.resolve(); fixtures=a.fixtures.resolve()
    manifest=json.loads((fixtures/'manifest.json').read_text())
    results=[]
    with tempfile.TemporaryDirectory(prefix='hachistep-conformance-') as directory:
        for case in manifest['cases']:
            image=fixtures/case['firmware']
            if hashlib.sha256(image.read_bytes()).hexdigest()!=case['sha256']:raise ValueError('fixture hash mismatch')
            out=Path(directory)/case['name']
            command=[str(runner),'run','--firmware',str(image),'--eeprom',str(fixtures/'blank-eeprom.bin'),
                     '--milliseconds',str(case['milliseconds']),'--out',str(out)]
            if case.get('input'):command+=['--input',str(fixtures/case['input'])]
            run=subprocess.run(command,text=True,capture_output=True,timeout=30)
            failures=[]
            if run.returncode:failures.append(run.stdout+run.stderr)
            else:
                report=json.loads((out/'report.json').read_text())
                for domain,start in [('ram',0xf780),('eeprom',0)]:
                    data=(out/f'{domain}.bin').read_bytes()
                    for address,hexbytes in case['expected'].get(domain,{}).items():
                        offset=int(address,16)-start;expected=bytes.fromhex(hexbytes)
                        actual=data[offset:offset+len(expected)]
                        if actual!=expected:failures.append(f'{domain}[{address}]: expected {expected.hex()}, got {actual.hex()}')
                for key in ['nv_commits','ir_events']:
                    if key in case['expected'] and report[key]!=case['expected'][key]:failures.append(f'{key}: {report[key]} != {case["expected"][key]}')
                if 'er0' in case['expected'] and report['er'][0]!=case['expected']['er0']:failures.append('ER0 mismatch')
            results.append({'case':case['name'],'passed':not failures,'failures':failures})
            print(('PASS' if not failures else 'FAIL')+' '+case['name'])
            for error in failures:print(error)
    summary={'schema':1,'kind':'software conformance; not hardware captures','results':results}
    if a.report:
        with a.report.open('x') as f:json.dump(summary,f,indent=2);f.write('\n')
    if not all(r['passed'] for r in results):raise SystemExit(1)
if __name__=='__main__':main()
