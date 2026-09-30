from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from compare_workloads import BYTE_KEYS, compare_case, counts, normalize, observe, paired_blocks, schedule


def report(**changes):
    value = {
        "ns": 100, "horizon_us": 1000, "calls": 1, "retired": 20,
        "pc": 0x100, "ccr": 0, "registers": [0] * 8, "ram_hash": 1,
        "lcd_hash": 2, "eeprom_hash": 3, "ssu_counts": [10, 0],
        "events": 2, "event_hash": 4, "sleeping": True, "completed": True,
    }
    return {**value, **changes}


class WorkloadComparison(unittest.TestCase):
    def run_case(self, variants, callback):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rom = root / "board_cpu.bin"
            rom.write_bytes(b"guest image")
            case = {"name": "board_cpu", "rom": str(rom), "mode": "job", "quantum_us": 1000, "limit_us": 1000000}
            with patch("compare_workloads.observe", side_effect=callback):
                return compare_case(case, variants, root / "result", 2, 0, 10)

    def test_mismatched_preflight_prevents_every_timing_run(self):
        phases = []

        def observe(variant, case, phase, directory, timeout):
            phases.append(phase)
            return {"report": report(pc=variant["pc"])}

        with self.assertRaisesRegex(ValueError, "preflight differs"):
            self.run_case({"a": {"pc": 1}, "b": {"pc": 2}}, observe)
        self.assertEqual(phases, ["preflight", "preflight"])

    def test_cross_core_difference_is_retained_without_equivalence_claim(self):
        def observe(variant, case, phase, directory, timeout):
            return {"report": report(retired=variant["retired"])}

        result = self.run_case({"hs": {"retired": 20, "group": "hs"}, "pw": {"retired": 19, "group": "pw"}}, observe)
        self.assertEqual(result["preflight"]["cross_group_differences"]["pw"]["retired"], {"reference": 20, "value": 19})
        self.assertEqual(result["summary"]["pw"]["throughput_medians"]["retired_per_second"], 190000000)
        self.assertEqual(len(result["raw_samples"]["hs"]), 4)

    def test_changed_timed_endpoint_is_rejected(self):
        def observe(variant, case, phase, directory, timeout):
            return {"report": report(ram_hash=2 if phase == "timing" else 1)}

        with self.assertRaisesRegex(ValueError, "timed endpoint differs"):
            self.run_case({"hs": {}}, observe)

    def test_work_is_separate_and_must_match_its_uninstrumented_variant(self):
        phases = []

        def observe(variant, case, phase, directory, timeout):
            phases.append(phase)
            value = report(ns=999999 if phase == "work" else 100)
            if phase == "work":
                value["work"] = {"cpu_phase_dispatches": 100, "interval_exits": {"request": 30}}
            return {"report": value}

        result = self.run_case({"hs": {"work_adapter": "accounting"}}, observe)
        self.assertEqual(phases[:2], ["preflight", "work"])
        self.assertEqual(result["summary"]["hs"]["median_ns"], 100)
        accounting = result["work"]["hs"]["accounting"]
        self.assertEqual(accounting["per"]["retired"]["cpu_phase_dispatches"], 5)
        self.assertEqual(accounting["per"]["ssu_tx_bytes"]["interval_exits.request"], 3)
        self.assertIsNone(accounting["per"]["ssu_rx_bytes"]["interval_exits.request"])

        def changed(variant, case, phase, directory, timeout):
            return {"report": report(retired=19 if phase == "work" else 20, work={"dispatches": 100})}

        with self.assertRaisesRegex(ValueError, "instrumented endpoint differs"):
            self.run_case({"hs": {"work_adapter": "accounting"}}, changed)

    def test_complete_history_difference_is_detected_with_equal_endpoints(self):
        phases = []

        def observe(variant, case, phase, directory, timeout):
            phases.append(phase)
            directory.mkdir()
            (directory / "state.bin").write_bytes(b"same native state")
            (directory / "events.txt").write_text(f"1 event {variant.get('event', 1)}\n2 event\n")
            return {"report": report(history={"path": "events.txt", "complete": True, "records": 2})}

        # A legacy adapter first in a group must not prevent comparison between
        # the following two artifact-capable adapters.
        with self.assertRaisesRegex(ValueError, "state/history preflight differs"):
            self.run_case({"legacy": {}, "a": {"artifacts": True}, "b": {"artifacts": True, "event": 2}}, observe)
        self.assertEqual(phases, ["preflight"] * 3)

    def test_truncated_history_is_rejected_before_timing(self):
        def observe(variant, case, phase, directory, timeout):
            return {"report": report(history={"path": "events.txt", "complete": False, "records": 2})}

        with self.assertRaisesRegex(ValueError, "complete=true"):
            self.run_case({"hs": {"artifacts": True}}, observe)

    def test_work_counters_reject_invalid_values_and_reset_denominator(self):
        for value in (-1, True, 0.5):
            with self.assertRaises(ValueError):
                counts({"dispatches": value})
        result = normalize({"dispatches": 20}, report(resets=1), BYTE_KEYS)
        self.assertIsNone(result["per"]["retired"]["dispatches"])
        self.assertIsNone(result["per"]["ssu_tx_bytes"]["dispatches"])
        cumulative = normalize({"dispatches": 20}, report(resets=1, cumulative_counts=True), BYTE_KEYS)
        self.assertEqual(cumulative["per"]["retired"]["dispatches"], 1)
        self.assertEqual(cumulative["per"]["ssu_tx_bytes"]["dispatches"], 2)

    def test_instrumented_reply_cannot_be_used_for_timing(self):
        import json
        import subprocess

        with tempfile.TemporaryDirectory() as directory:
            process = subprocess.CompletedProcess([], 0, json.dumps(report(work={"dispatches": 1})), "")
            case = {"mode": "time", "rom": "image", "quantum_us": 1, "limit_us": 10}
            with patch("compare_workloads.subprocess.run", return_value=process):
                with self.assertRaisesRegex(ValueError, "instrumented reply"):
                    observe({"adapter": "probe"}, case, "timing", Path(directory) / "run", 10)

    def test_paired_schedule_varies_order_and_balances_each_block(self):
        names = ["a", "b", "c"]
        value = schedule(names, 5, 7)
        blocks = [value[start:start + 6] for start in range(0, len(value), 6)]
        self.assertGreater(len({tuple(name for _, name in block) for block in blocks}), 1)
        for block in blocks:
            order = [name for _, name in block]
            self.assertEqual(order[:3], order[3:][::-1])
            self.assertEqual(sorted(order), sorted(names * 2))

    def test_longer_blocks_retain_samples_and_exclude_process_setup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            rom = root / "board_cpu.bin"
            rom.write_bytes(b"guest image")
            case = {"name": "board_cpu", "rom": str(rom), "mode": "job", "quantum_us": 1000, "limit_us": 1000000}
            def observe(variant, case, phase, directory, timeout):
                return {"report": report(ns=variant["ns"]), "process_wall_seconds": variant["setup"]}
            with patch("compare_workloads.observe", side_effect=observe):
                result = compare_case(case, {"a": {"ns": 100, "setup": 1000}, "b": {"ns": 50, "setup": 9000}}, root / "result", 3, 7, 10, block_runs=4)
            for name in ["a", "b"]:
                samples = result["raw_samples"][name]
                self.assertEqual(len(samples), 24)
                self.assertEqual([row["order_index"] for row in samples], sorted(row["order_index"] for row in samples))
                for repeat in range(3):
                    paired = [row for row in samples if row["repeat"] == repeat]
                    self.assertEqual(len(paired), 8)
                    self.assertEqual(sorted(row["block_run"] for row in paired), [0, 0, 1, 1, 2, 2, 3, 3])
                    self.assertEqual(len({row["slot"] for row in paired}), 2)
            self.assertEqual([row["time_ratio_to_baseline"]["b"] for row in result["paired_blocks"]], [0.5, 0.5, 0.5])
            self.assertEqual(result["summary"]["a"]["median_ns"], 100)
            self.assertEqual(result["summary"]["b"]["median_ns"], 50)

    def test_paired_ratios_preserve_change_and_spread_across_host_drift(self):
        rows = {
            "before": [
                {"repeat": repeat, "report": {"ns": ns}}
                for repeat, ns in [(0, 100), (0, 120), (1, 1000), (1, 1200)]
            ],
            "after": [
                {"repeat": repeat, "report": {"ns": ns}}
                for repeat, ns in [(0, 50), (0, 60), (1, 700), (1, 840)]
            ],
        }
        blocks = paired_blocks(rows, "before")
        self.assertEqual([block["time_ratio_to_baseline"]["after"] for block in blocks], [0.5, 0.7])
        rows["after"].pop()
        with self.assertRaisesRegex(ValueError, "two samples"):
            paired_blocks(rows, "before")


if __name__ == "__main__":
    unittest.main()
