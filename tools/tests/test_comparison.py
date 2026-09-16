from pathlib import Path
import json
import tempfile
import unittest
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from compare_runs import compare, EXPORTS, REPORT_KEYS

class Comparison(unittest.TestCase):
    def fixture(self, root):
        paths=[root/'left',root/'right']
        report={key:0 for key in REPORT_KEYS}
        report.update(fault=None,time_raw='100',requested_time_raw='100',events=2,
                      trace_records=2,trace_dropped=0,trace_complete=True)
        for path in paths:
            path.mkdir()
            for name in EXPORTS:(path/name).write_bytes(b'abc')
            (path/'events.txt').write_bytes(b'1\tfirst\n2\tsecond\n')
            (path/'report.json').write_text(json.dumps(report))
        return paths
    def test_equal_endpoints_do_not_hide_a_different_history(self):
        with tempfile.TemporaryDirectory() as d:
            a,b=self.fixture(Path(d))
            self.assertTrue(compare(a,b,a/'events.txt',b/'events.txt')['equivalent'])
            (b/'events.txt').write_bytes(b'1\twrong\n2\tsecond\n')
            result=compare(a,b,a/'events.txt',b/'events.txt')
            self.assertFalse(result['equivalent'])
            self.assertEqual(result['history']['first_different_record'],0)
            self.assertTrue(compare(a,b)['equivalent']) # endpoints alone do not observe that change
    def test_truncated_histories_cannot_pass(self):
        with tempfile.TemporaryDirectory() as d:
            a,b=self.fixture(Path(d));(b/'events.txt').write_bytes(b'1\tfirst\n')
            with self.assertRaisesRegex(ValueError,'history length'):compare(a,b,a/'events.txt',b/'events.txt')
    def test_reports_and_exported_bytes_both_matter(self):
        with tempfile.TemporaryDirectory() as d:
            a,b=self.fixture(Path(d));(b/'ram.bin').write_bytes(b'abd')
            r=json.loads((b/'report.json').read_text());r['interrupt_entries']=1
            (b/'report.json').write_text(json.dumps(r))
            result=compare(a,b)
            self.assertIn('ram.bin: first difference at byte 2',result['differences'])
            self.assertFalse(result['equivalent'])
