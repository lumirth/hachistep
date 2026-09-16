from __future__ import annotations
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile
TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
from import_inputs import import_images, ROM_MEMBER
from preview import read_pgm, render
from render_audio import samples, load, ONE

class HostTools(unittest.TestCase):
    def test_import_only_expected_member_and_never_overwrites(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            with zipfile.ZipFile(root/'inputs.zip', 'w') as z:
                z.writestr(ROM_MEMBER, b'R' * 49152)
                z.writestr('../escape.txt', 'must not extract')
            (root/'eeprom.bin').write_bytes(b'E'*65536)
            result = import_images(root/'inputs.zip', root/'eeprom.bin', root/'out')
            self.assertEqual((root/'out/pokewalker.bin').stat().st_size, 49152)
            self.assertFalse((root/'escape.txt').exists())
            self.assertTrue(result['private_inputs'])
            with self.assertRaises(FileExistsError):
                import_images(root/'inputs.zip', root/'eeprom.bin', root/'out')
    def test_bad_input_leaves_no_destination(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root/'eeprom.bin').write_bytes(b'bad')
            with self.assertRaises(ValueError):
                import_images(root/'missing.zip', root/'eeprom.bin', root/'out')
            self.assertFalse((root/'out').exists())
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
