from __future__ import annotations
import json
from pathlib import Path
import sys
import tempfile
import unittest
TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
from preview import read_pgm, render
from render_audio import samples, load, ONE

class HostTools(unittest.TestCase):
    def test_pgm_preserves_whitespace_valued_pixels(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            frame = root/'capture.pgm'
            frame.write_bytes(b'P5\n2 2\n255\n'+bytes([10,13,32,255]))
            self.assertEqual(read_pgm(frame), (2,2,bytes([10,13,32,255])))
            render([frame], root/'view.html')
            self.assertIn('Captured firmware output', (root/'view.html').read_text())
            with self.assertRaises(FileExistsError):
                render([frame], root/'view.html')
    def test_audio_integrates_transitions_instead_of_aliasing_edge_samples(self):
        # Half each first sample positive/negative, then neutral.
        edge = ONE//2000
        data = list(samples(ONE//1000, [(0,1),(edge,-1)], 1000, 10000))
        self.assertEqual(data, [0])
        self.assertEqual(list(samples(ONE//1000, [], 1000, 10000)), [0])
    def test_audio_refuses_truncation(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d)
            (root/'report.json').write_text(json.dumps({'trace_complete':False,'trace_dropped':12}))
            with self.assertRaises(ValueError):
                load(root/'trace.txt', root/'report.json')
if __name__ == '__main__':
    unittest.main()
