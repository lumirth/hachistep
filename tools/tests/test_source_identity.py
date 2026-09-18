"""Run receipts must distinguish different contents at the same dirty revision."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from _support import source_identity, write_summary


class SourceIdentity(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.git("init", "-q")
        (self.root / ".gitignore").write_text("out/\n")
        (self.root / "source").write_bytes(b"original")
        self.git("add", ".")
        self.git(
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "fixture",
        )

    def git(self, *args):
        return subprocess.check_output(
            ["git", "-C", str(self.root), *args], stderr=subprocess.DEVNULL
        )

    def test_same_dirty_paths_with_different_bytes_have_different_identities(self):
        file = self.root / "source"
        file.write_bytes(b"first")
        before = source_identity(self.root)
        file.write_bytes(b"other")
        after = source_identity(self.root)
        self.assertEqual(before["commit"], after["commit"])
        self.assertEqual(before["changes"], after["changes"])
        self.assertNotEqual(before["tree_sha256"], after["tree_sha256"])

        (self.root / "new file\n.bin").write_bytes(b"\x00\xff")
        untracked = source_identity(self.root)
        self.assertNotEqual(after["tree_sha256"], untracked["tree_sha256"])
        (self.root / "out").mkdir()
        (self.root / "out/report").write_text("ignored")
        self.assertEqual(
            untracked["tree_sha256"], source_identity(self.root)["tree_sha256"]
        )
        self.assertEqual(
            untracked["tree_sha256"], source_identity(self.root / "out")["tree_sha256"]
        )

    def test_modes_symlink_targets_and_deletions_affect_identity(self):
        previous = source_identity(self.root)["tree_sha256"]
        actions = [
            lambda: (self.root / "source").chmod(0o755),
            lambda: (self.root / "link").symlink_to("source"),
            lambda: (self.root / "source").unlink(),
        ]
        for action in actions:
            action()
            current = source_identity(self.root)["tree_sha256"]
            self.assertIsNotNone(current)
            self.assertNotEqual(previous, current)
            previous = current

    def test_changed_source_receipt_is_saved_before_failure(self):
        before = source_identity(self.root)
        (self.root / "source").write_text("changed during run")
        after = source_identity(self.root)
        report = self.root / "report.json"
        with patch("_support.source_identity", return_value=after):
            with self.assertRaisesRegex(RuntimeError, "source changed"):
                write_summary(report, {"steps": ["completed"]}, before)
        saved = json.loads(report.read_text())
        self.assertEqual(saved["source"], before)
        self.assertEqual(saved["source_after"], after)
        self.assertFalse(saved["source_unchanged"])
        self.assertEqual(saved["steps"], ["completed"])

    def test_no_git_metadata_is_unknown_not_clean(self):
        with tempfile.TemporaryDirectory() as directory:
            source = source_identity(Path(directory))
            self.assertIsNone(source["tree_sha256"])
            self.assertIsNone(source["dirty"])
            with patch("_support.source_identity", return_value=source):
                path = Path(directory) / "summary.json"
                write_summary(path, {}, source)
                self.assertIsNone(json.loads(path.read_text())["source_unchanged"])
