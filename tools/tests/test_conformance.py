from __future__ import annotations
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('suite_adapter', ROOT/'conformance/run.py')
assert spec and spec.loader
suite = importlib.util.module_from_spec(spec)
spec.loader.exec_module(suite)

class Conformance(unittest.TestCase):
    def test_rejects_misspelled_and_out_of_range_expectations(self):
        for expected in [{'interrupt_entry':1}, {'ram':{'0000':'01'}}, {'ram':{'ff7f':'abcd'}},
                         {'eeprom':{'ffff':'0102'}}, {'ram':{'f800':''}}, {'er0':1<<32},
                         {'display_on':1}, {'ir_events':True}]:
            with self.subTest(expected=expected), self.assertRaises(ValueError):
                suite.validate_expected(expected)
        suite.validate_expected({'ram':{'f780':'00','ff7f':'ab'},'interrupt_entries':1})

    def test_all_firmware_timeline_and_eeprom_inputs_are_hashed(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d)/'fixtures'
            subprocess.run([sys.executable,str(ROOT/'conformance/build.py'),str(root)],check=True,capture_output=True)
            manifest=suite.load_manifest(root)
            self.assertEqual(len(manifest['cases']),15)
            for name in ['blank-eeprom.bin','register-aliases.bin','nmi-masked-sleep.csv']:
                path=root/name;data=path.read_bytes()
                path.write_bytes(bytes([data[0]^1])+data[1:])
                with self.subTest(name=name),self.assertRaises(ValueError):suite.load_manifest(root)
                path.write_bytes(data)
            self.assertEqual(suite.load_manifest(root),manifest)

    def test_fixture_paths_cannot_escape_the_selected_corpus(self):
        with tempfile.TemporaryDirectory() as d:
            for name in ['../other.bin','/tmp/file','.', '', 'a/b']:
                with self.subTest(name=name),self.assertRaises(ValueError):
                    suite.checked_file(Path(d),name,'0'*64)

    def test_new_interrupt_expectations_are_actually_compared(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);(root/'ram.bin').write_bytes(bytes(2048));(root/'eeprom.bin').write_bytes(bytes(65536))
            raw=(8<<64)//1000
            report={'fault':None,'time_raw':str(raw),'requested_time_raw':str(raw),'time_us':7999,
                    'interrupt_entries':0,'er':[0]*8}
            failures=suite.compare({'interrupt_entries':1},report,root,8)
            self.assertEqual(failures,['interrupt_entries: expected 1, got 0'])
            report['interrupt_entries']=1
            self.assertEqual(suite.compare({'interrupt_entries':1},report,root,8),[])
            report['time_raw']=str(raw-1)
            self.assertEqual(suite.compare({'interrupt_entries':1},report,root,8),
                             ['requested exclusive horizon was not reached'])

    def test_unknown_or_unmeasured_expectations_cannot_be_labeled_hardware(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d)/'fixtures'
            subprocess.run([sys.executable,str(ROOT/'conformance/build.py'),str(root)],check=True,capture_output=True)
            path=root/'manifest.json';data=json.loads(path.read_text())
            data['cases'][0]['expectation']['kind']='hardware_measured'
            path.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError,'captured observation'):suite.load_manifest(root)
