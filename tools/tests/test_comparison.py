import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from compare_runs import EXPORTS, REPORT_KEYS, compare, compare_resume


class Comparison(unittest.TestCase):
    def test_restoration_compares_complete_history_and_causal_state(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            whole, before = self.fixture(root)
            resumed = root / "resumed"
            resumed.mkdir()
            for name in EXPORTS:
                (resumed / name).write_bytes(b"abc")
            report = json.loads((whole / "report.json").read_text())
            report["start_time_raw"] = "0"
            (whole / "report.json").write_text(json.dumps(report))
            prefix = {
                **report,
                "time_raw": "50",
                "requested_time_raw": "50",
                "events": 1,
                "trace_records": 1,
            }
            suffix = {**report, "start_time_raw": "50", "events": 1, "trace_records": 1}
            (before / "report.json").write_text(json.dumps(prefix))
            (resumed / "report.json").write_text(json.dumps(suffix))
            (before / "events.txt").write_bytes(b"1\tfirst\n")
            (resumed / "events.txt").write_bytes(b"2\tsecond\n")
            self.assertTrue(compare_resume(whole, before, resumed)["equivalent"])
            (resumed / "events.txt").write_bytes(b"2\twrong\n")
            self.assertFalse(compare_resume(whole, before, resumed)["equivalent"])
            (resumed / "events.txt").write_bytes(b"2\tsecond\n")
            (resumed / "state.bin").write_bytes(b"different clock phase")
            self.assertFalse(compare_resume(whole, before, resumed)["equivalent"])
            (before / "events.txt").write_bytes(b"")
            with self.assertRaisesRegex(ValueError, "history length"):
                compare_resume(whole, before, resumed)

    def fixture(self, root):
        paths = [root / "left", root / "right"]
        report = {key: 0 for key in REPORT_KEYS}
        report.update(
            fault=None,
            time_raw="100",
            requested_time_raw="100",
            events=2,
            trace_records=2,
            trace_dropped=0,
            trace_complete=True,
        )
        for path in paths:
            path.mkdir()
            for name in EXPORTS:
                (path / name).write_bytes(b"abc")
            (path / "events.txt").write_bytes(b"1\tfirst\n2\tsecond\n")
            (path / "report.json").write_text(json.dumps(report))
        return paths

    def test_equal_endpoints_do_not_hide_a_different_history(self):
        with tempfile.TemporaryDirectory() as d:
            a, b = self.fixture(Path(d))
            self.assertTrue(
                compare(a, b, a / "events.txt", b / "events.txt")["equivalent"]
            )
            (b / "events.txt").write_bytes(b"1\twrong\n2\tsecond\n")
            result = compare(a, b, a / "events.txt", b / "events.txt")
            self.assertFalse(result["equivalent"])
            self.assertEqual(result["history"]["first_different_record"], 0)
            self.assertTrue(
                compare(a, b)["equivalent"]
            )  # endpoints alone do not observe that change

    def test_truncated_histories_cannot_pass(self):
        with tempfile.TemporaryDirectory() as d:
            a, b = self.fixture(Path(d))
            (b / "events.txt").write_bytes(b"1\tfirst\n")
            with self.assertRaisesRegex(ValueError, "history length"):
                compare(a, b, a / "events.txt", b / "events.txt")

    def test_reports_and_exported_bytes_both_matter(self):
        with tempfile.TemporaryDirectory() as d:
            a, b = self.fixture(Path(d))
            (b / "ram.bin").write_bytes(b"abd")
            r = json.loads((b / "report.json").read_text())
            r["interrupt_entries"] = 1
            (b / "report.json").write_text(json.dumps(r))
            result = compare(a, b)
            self.assertIn("ram.bin: first difference at byte 2", result["differences"])
            self.assertFalse(result["equivalent"])
