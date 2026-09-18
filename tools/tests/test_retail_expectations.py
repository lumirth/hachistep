import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from _support import ROOT, digest
from compare_runs import OBSERVATION_FILES, REPORT_KEYS
from retail_expectations import check, load


class RetailExpectations(unittest.TestCase):
    def test_current_baseline_is_complete_and_not_hardware_evidence(self):
        path = ROOT / "workloads/retail.json"
        data = json.loads(path.read_text())
        result = load(path, data["inputs"])
        self.assertFalse(result["hardware_measured"])
        with self.assertRaisesRegex(ValueError, "inputs do not match"):
            load(path, {"firmware": "wrong", "eeprom": "wrong"})

    def test_frame_and_timing_changes_are_not_hidden_by_successful_execution(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            for name in OBSERVATION_FILES:
                (root / name).write_bytes(b"valid fixture")
            report = {key: 0 for key in REPORT_KEYS}
            report["fault"] = None
            case = {
                "milliseconds": 10,
                "input_sha256": None,
                "report": copy.deepcopy(report),
                "exports": {name: digest(root / name) for name in OBSERVATION_FILES},
            }
            self.assertEqual(check(case, 10, None, root, report), [])
            (root / "frame.pgm").write_bytes(b"wrong pixels")
            report["interrupt_entries"] = 1
            failures = check(case, 10, None, root, report)
            self.assertEqual(len(failures), 2)
            self.assertTrue(any("frame.pgm" in s for s in failures))
            self.assertTrue(any("interrupt_entries" in s for s in failures))

    def test_changed_scenario_and_duration_cannot_use_the_same_expectation(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            for name in OBSERVATION_FILES:
                (root / name).write_bytes(b"fixture")
            timeline = root / "scenario.csv"
            timeline.write_text("1,button,1\n")
            case = {
                "milliseconds": 10,
                "input_sha256": None,
                "report": {},
                "exports": {name: digest(root / name) for name in OBSERVATION_FILES},
            }
            result = check(case, 11, timeline, root, {})
            self.assertEqual(len(result), 2)

    def test_missing_semantic_fields_rejected(self):
        data = json.loads((ROOT / "workloads/retail.json").read_text())
        del data["cases"]["home"]["report"]["er"]
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "wrong.json"
            path.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, "semantic report"):
                load(path, data["inputs"])
