"""Check HachiStep's translation to the independent observation contract."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
from hachiware_adapter import export, observations


class Adapter(unittest.TestCase):
    def test_completion_uses_exact_core_time_and_exports_requested_scalars(self):
        end = (8 << 64) // 1000
        report = {
            "fault": None,
            "time_raw": str(end),
            "requested_time_raw": str(end),
            "time_us": end * 1_000_000 >> 64,
            "er": [0x1234] * 8,
        }
        result = observations(report, 8, ["er0"])
        self.assertEqual(result, {"fault": None, "completed": True, "er0": 0x1234})
        report["time_raw"] = str(end - 1)
        self.assertFalse(observations(report, 8, [])["completed"])
        report["time_raw"] = str(end)
        report["requested_time_raw"] = str(end + 1)
        self.assertFalse(observations(report, 8, [])["completed"])

    def test_requested_exports_preserve_controller_bytes_and_logical_shades(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            (out / "lcd-ram.bin").write_bytes(bytes(range(256)) * 16)
            export(out, ["lcd"])
            self.assertEqual(
                (out / "lcd.bin").read_bytes(), (out / "lcd-ram.bin").read_bytes()
            )
            (out / "frame.pgm").write_bytes(
                b"P5\n96 64\n255\n" + bytes([255, 170, 85, 0]) * 1536
            )
            export(out, ["pixels"])
            self.assertEqual(
                (out / "pixels.bin").read_bytes(), bytes([0, 1, 2, 3]) * 1536
            )
            with self.assertRaises(FileExistsError):
                export(out, ["pixels"])

    def test_description_needs_no_firmware_or_executable(self):
        result = subprocess.run(
            [sys.executable, str(TOOLS / "hachiware_adapter.py"), "--describe"],
            capture_output=True,
            text=True,
            check=True,
        )
        caps = json.loads(result.stdout)
        self.assertEqual(caps["target"], "H8/38606F")
        self.assertIn("digital:p31", caps["inputs"])
        self.assertIn("ram", caps["observations"])
