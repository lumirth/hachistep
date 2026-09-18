from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
from _support import cli_command
from preview import read_pgm, render
from render_audio import ONE, load, samples


class HostTools(unittest.TestCase):
    def test_native_and_wasi_launches_preserve_arguments_and_file_access(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            images, results = root / "images", root / "results"
            images.mkdir()
            results.mkdir()
            arguments = [
                "run",
                "--firmware",
                str(images / "image.bin"),
                "--out",
                str(results / "new"),
                "--trace",
                str(results / "new/events.txt"),
            ]
            with patch("_support.binary", return_value="wasmtime") as runtime:
                native = root / "native"
                self.assertEqual(
                    cli_command(native, arguments), [str(native), *arguments]
                )
                runtime.assert_not_called()
                module = root / "core.wasm"
                self.assertEqual(
                    cli_command(module, arguments),
                    [
                        "wasmtime",
                        "run",
                        "--dir",
                        str(images),
                        "--dir",
                        str(results),
                        str(module),
                        *arguments,
                    ],
                )
                runtime.assert_called_once_with("wasmtime")

    def test_pgm_preserves_whitespace_valued_pixels(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            frame = root / "capture.pgm"
            frame.write_bytes(b"P5\n2 2\n255\n" + bytes([10, 13, 32, 255]))
            self.assertEqual(read_pgm(frame), (2, 2, bytes([10, 13, 32, 255])))
            render([frame], root / "view.html")
            self.assertIn("Captured firmware output", (root / "view.html").read_text())
            with self.assertRaises(FileExistsError):
                render([frame], root / "view.html")

    def test_audio_integrates_transitions_instead_of_aliasing_edge_samples(self):
        # Half each first sample positive/negative, then neutral.
        edge = ONE // 2000
        data = list(samples(ONE // 1000, [(0, 1), (edge, -1)], 1000, 10000))
        self.assertEqual(data, [0])
        self.assertEqual(list(samples(ONE // 1000, [], 1000, 10000)), [0])

    def test_audio_refuses_truncation(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "report.json").write_text(
                json.dumps({"trace_complete": False, "trace_dropped": 12})
            )
            with self.assertRaises(ValueError):
                load(root / "trace.txt", root / "report.json")


if __name__ == "__main__":
    unittest.main()
