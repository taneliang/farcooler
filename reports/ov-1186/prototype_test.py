#!/usr/bin/env python3
"""Counterexamples for the branch-only archive consumer."""
import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("prototype", Path(__file__).with_name("prototype.py"))
p = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p)


class ArchiveTests(unittest.TestCase):
    def test_exact_bytes_and_platform_accept(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "archive"
            archive.write_bytes(b"real archive bytes")
            manifest = {"schema": 1, "package": p.PACKAGE, "target": p.TARGET,
                        "source_key": "source", "archive_sha256": p.sha(archive.read_bytes()),
                        "nextest": p.NEXTEST}
            p.validate(manifest, archive, "Darwin", "arm64", "source")
            for field, value in [("source_key", "other"), ("target", "x86_64-unknown-linux-gnu"),
                                 ("package", "other"), ("schema", 2), ("nextest", "old"),
                                 ("archive_sha256", "wrong")]:
                bad = copy.deepcopy(manifest)
                bad[field] = value
                with self.subTest(field=field), self.assertRaises(ValueError):
                    p.validate(bad, archive, "Darwin", "arm64", "source")
            for system, machine in [("Linux", "arm64"), ("Darwin", "x86_64")]:
                with self.subTest(system=system, machine=machine), self.assertRaises(ValueError):
                    p.validate(manifest, archive, system, machine, "source")
            archive.write_bytes(b"corrupted")
            with self.assertRaisesRegex(ValueError, "corruption"):
                p.validate(manifest, archive, "Darwin", "arm64", "source")

    def test_working_bytes_modes_and_untracked_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.rs"
            source.write_text("old")
            with patch.object(p, "ROOT", root), patch.object(p, "output", side_effect=["source.rs", ""]):
                first = p.source_key()
            source.write_text("changed")
            with patch.object(p, "ROOT", root), patch.object(p, "output", side_effect=["source.rs", ""]):
                self.assertNotEqual(first, p.source_key())
            with patch.object(p, "ROOT", root), patch.object(p, "output", side_effect=["source.rs", "new.rs"]):
                with self.assertRaisesRegex(ValueError, "untracked"):
                    p.source_key()


if __name__ == "__main__":
    unittest.main()
