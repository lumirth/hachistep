from __future__ import annotations

import contextlib
import hashlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check


class CheckSelection(unittest.TestCase):
    @contextlib.contextmanager
    def checkout(self):
        with tempfile.TemporaryDirectory() as directory, contextlib.ExitStack() as stack:
            root = Path(directory).resolve()
            suite = root / "suite"
            suite.mkdir()
            for name in ("build.py", "run.py"):
                (suite / name).touch()
            tests = root / "crates/hs-core/tests"
            tests.mkdir(parents=True)
            for name in ("serial_appointments", "sensor_spi"):
                (tests / f"{name}.rs").touch()
            release = root / "target/release/hachistep"
            release.parent.mkdir(parents=True)
            release.write_bytes(b"release candidate")
            iteration = root / "target/iteration/hachistep"
            iteration.parent.mkdir()
            iteration.write_bytes(b"iteration candidate")
            calls = []

            def run(command, output, name, **kwargs):
                calls.append((name, command))
                if name.startswith("focused-test"):
                    count = command.count("--test") + int("--lib" in command)
                    (output / f"{name}.log").write_text("test result: ok. 1 passed;\n" * count)
                return {"name": name, "command": command, "returncode": 0}

            for name, value in (
                ("ROOT", root),
                ("binary", lambda name: name),
                ("run", run),
                ("source_identity", lambda: {"tree_sha256": "checkout"}),
                ("versions", lambda: {}),
            ):
                stack.enter_context(patch.object(check, name, value))
            resolver = stack.enter_context(patch.object(check, "release_executable", return_value=release))
            summary = stack.enter_context(patch.object(check, "write_summary"))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            yield root, suite, calls, resolver, summary

    def test_default_still_runs_full_matrices_and_all_guest_cases(self):
        with self.checkout() as (root, suite, calls, resolver, summary):
            check.main(["--hachiware", str(suite)])
            commands = dict(calls)
            self.assertIn("--workspace", commands["tests"])
            self.assertIn("--release", commands["release-tests"])
            self.assertIn("--all-features", commands["trace-tests"])
            self.assertIn("--all-targets", commands["clippy"])
            self.assertNotIn("--case", commands["fixture-build"])
            self.assertIn(str(root / "target/release/hachistep"), commands["conformance"])
            self.assertEqual(summary.call_args.args[1]["coverage"], "full")
            resolver.assert_called_once()

    def test_execution_uses_optimized_asserting_tests_and_matching_cli(self):
        with self.checkout() as (root, suite, calls, resolver, summary):
            check.main(["--execution", "--hachiware", str(suite)])
            commands = dict(calls)
            tests = commands["execution-tests"]
            self.assertEqual(tests[tests.index("--profile") + 1], "iteration")
            self.assertIn("--lib", tests)
            for target in ("cpu_regressions", "register_access", "serial_appointments", "save_state", "interrupt_admission"):
                self.assertIn(target, tests)
            self.assertFalse({"tests", "release-tests", "trace-tests"} & commands.keys())
            self.assertNotIn("--all-targets", commands["clippy"])
            self.assertNotIn("--all-features", commands["clippy"])
            self.assertIn("irq-source-clear-cancels", commands["fixture-build"])
            self.assertIn("prefetch-before-self-modifying-store", commands["fixture-build"])
            self.assertIn(str(root / "target/iteration/hachistep"), commands["conformance"])
            report = summary.call_args.args[1]
            self.assertEqual(report["coverage"], "execution")
            self.assertTrue(report["runner_unchanged"])
            self.assertEqual(report["runner"]["sha256"], hashlib.sha256(b"iteration candidate").hexdigest())

    def test_supplied_candidate_skips_cli_build_but_checks_current_rust_sources(self):
        with self.checkout() as (root, suite, calls, resolver, summary):
            candidate = root / "preserved-candidate"
            candidate.write_bytes(b"preserved candidate")
            check.main(["--execution", "--runner", str(candidate), "--hachiware", str(suite)])
            commands = dict(calls)
            self.assertIn("execution-tests", commands)
            self.assertIn("clippy", commands)
            self.assertFalse(any("build" in command[1:2] for _, command in calls))
            self.assertIn(str(candidate), commands["conformance"])
            self.assertTrue(summary.call_args.args[1]["runner"]["supplied"])
            resolver.assert_not_called()

    def test_wasi_candidate_is_rejected_before_creating_outputs_or_running_checks(self):
        with self.checkout() as (root, suite, calls, resolver, summary):
            module = root / "candidate.wasm"
            module.touch()
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                check.main(["--execution", "--runner", str(module), "--hachiware", str(suite)])
            self.assertEqual(calls, [])
            self.assertFalse((root / "out").exists())
            summary.assert_not_called()

    def test_runner_change_saves_identity_failure_instead_of_claiming_success(self):
        with self.checkout() as (root, suite, calls, resolver, summary):
            candidate = root / "preserved-candidate"
            candidate.write_bytes(b"original candidate")
            original_run = check.run

            def change_runner(command, output, name, **kwargs):
                record = original_run(command, output, name, **kwargs)
                if name == "conformance":
                    candidate.write_bytes(b"changed candidate")
                return record

            with patch.object(check, "run", change_runner), self.assertRaisesRegex(RuntimeError, "runner changed"):
                check.main(["--execution", "--runner", str(candidate), "--hachiware", str(suite)])
            self.assertFalse(summary.call_args.args[1]["runner_unchanged"])

    def test_failed_step_keeps_source_and_timing_receipt_and_stops_later_checks(self):
        with self.checkout() as (root, suite, calls, resolver, summary):
            original_run = check.run

            def fail_tests(command, output, name, **kwargs):
                if name == "execution-tests":
                    (output / f"{name}-failure.json").write_text(json.dumps({
                        "name": name, "command": command,
                        "returncode": 101, "wall_seconds": 2.5,
                    }))
                    raise RuntimeError("execution assertion failed")
                return original_run(command, output, name, **kwargs)

            with patch.object(check, "run", fail_tests), self.assertRaisesRegex(RuntimeError, "execution assertion failed"):
                check.main(["--execution", "--hachiware", str(suite)])
            report = summary.call_args.args[1]
            self.assertEqual(report["failure"], "execution assertion failed")
            self.assertEqual([step["name"] for step in report["steps"]], ["format", "execution-tests"])
            self.assertEqual(report["steps"][1]["returncode"], 101)
            self.assertEqual(report["steps"][1]["wall_seconds"], 2.5)
            self.assertEqual(summary.call_args.args[2], {"tree_sha256": "checkout"})
            self.assertIsNone(report["runner"])
            resolver.assert_not_called()

    def test_exact_rust_only_runs_the_selected_harness_and_needs_no_suite(self):
        with self.checkout() as (root, suite, calls, resolver, summary):
            check.main(["--test", "serial_appointments::lcd_stop", "--hachiware", str(root / "missing")])
            self.assertEqual(len(calls), 1)
            name, command = calls[0]
            self.assertEqual(command[command.index("--test") + 1], "serial_appointments")
            self.assertEqual(command[-3:], ["--", "lcd_stop", "--exact"])
            self.assertNotIn("--lib", command)
            self.assertEqual(summary.call_args.args[1]["coverage"], "focused")
            self.assertIsNone(summary.call_args.args[1]["runner"])
            resolver.assert_not_called()

    def test_whole_targets_batch_requested_harnesses_without_repeating_exact_tests(self):
        with self.checkout() as (root, suite, calls, resolver, summary):
            check.main(["--test", "lib", "--test", "sensor_spi", "--test", "sensor_spi::covered", "--features", "profile-work"])
            self.assertEqual(len(calls), 1)
            command = calls[0][1]
            self.assertIn("--lib", command)
            self.assertIn("sensor_spi", command)
            self.assertIn("profile-work", command)
            self.assertNotIn("covered", command)
            resolver.assert_not_called()

    def test_zero_match_and_ignored_only_selections_fail_and_stop_guest_checks(self):
        for result in ("test result: ok. 0 passed; 0 failed; 0 ignored;",
                       "test result: ok. 0 passed; 0 failed; 1 ignored;"):
            with self.subTest(result=result), self.checkout() as (root, suite, calls, resolver, summary):
                original_run = check.run

                def zero_matches(command, output, name, **kwargs):
                    record = original_run(command, output, name, **kwargs)
                    (output / f"{name}.log").write_text(result)
                    return record

                with patch.object(check, "run", zero_matches), self.assertRaisesRegex(RuntimeError, "ran no tests"):
                    check.main(["--test", "lib::missing", "--case", "infrared-transmit", "--hachiware", str(suite)])
                self.assertEqual(len(calls), 1)
                self.assertIn("ran no tests", summary.call_args.args[1]["failure"])
                self.assertIn("validation_error", summary.call_args.args[1]["steps"][0])

    def test_guest_only_with_supplied_runner_skips_rust_build_and_all_other_checks(self):
        with self.checkout() as (root, suite, calls, resolver, summary):
            candidate = root / "target/iteration/hachistep"
            with patch.object(check, "binary", side_effect=AssertionError("unexpected toolchain lookup")):
                check.main(["--case", "infrared-*", "--runner", str(candidate), "--hachiware", str(suite)])
            self.assertEqual([name for name, _ in calls], ["fixture-build", "conformance"])
            self.assertIn("infrared-*", calls[0][1])
            self.assertIn(str(candidate), calls[1][1])
            resolver.assert_not_called()

    def test_ambiguous_or_incomplete_focused_options_are_rejected(self):
        for arguments in (["--execution", "--test", "lib"], ["--features", "trace"],
                          ["--test", "lib", "--runner", "unused"], ["--test", "lib::"],
                          ["--test", "missing"], ["--test", "lib", "--features", "typo"]):
            with self.subTest(arguments=arguments), self.checkout() as (root, suite, calls, resolver, summary):
                with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                    check.main(arguments)
                self.assertFalse(calls)
                summary.assert_not_called()


if __name__ == "__main__":
    unittest.main()
