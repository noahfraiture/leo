"""The portable pack must preserve case identity without access to the original files."""

import hashlib
import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from benchmarks.leo_bench.cases import Case, Image, Question, fingerprint
from benchmarks.leo_bench.pack import export_pack, load_pack, pack_names


class PackTests(unittest.TestCase):
    def setUp(self):
        temporary = TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.original = self.directory / "recording-name.jpg"
        data = b"\xff\xd8\xff\xe0sample"
        self.original.write_bytes(data)
        self.digest = hashlib.sha256(data).hexdigest()
        image = Image(self.original, self.digest, "0 seconds")
        self.cases = [Case("opening", "What color?", (image,), (Question("color", ("red", "blue"), "blue"),))]
        self.pack = self.directory / "pack"

    def test_relocated_suites_reuse_the_same_bytes_and_keep_answers_in_a_separate_file(self):
        export_pack(self.pack, {"first": self.cases, "second": self.cases})
        moved = self.pack.rename(self.directory / "moved")
        self.original.unlink()

        self.assertEqual(moved.stat().st_mode & 0o077, 0)
        self.assertEqual(pack_names(moved), ["first", "second"])
        self.assertEqual([p.name for p in (moved / "images").iterdir()], [self.digest])
        inputs = json.loads((moved / "inputs/first.json").read_bytes())
        self.assertEqual(inputs[0]["questions"], [{"id": "color", "choices": ["red", "blue"]}])
        self.assertNotIn(str(self.original), json.dumps(inputs))
        self.assertEqual(fingerprint(load_pack(moved, "second")), fingerprint(self.cases))

    def test_rejects_missing_or_modified_image_bytes(self):
        export_pack(self.pack, {"first": self.cases})
        image = self.pack / "images" / self.digest
        image.write_bytes(b"bad frame")
        with self.assertRaisesRegex(ValueError, "Frame differs"):
            load_pack(self.pack, "first")
        image.unlink()
        with self.assertRaises(FileNotFoundError):
            load_pack(self.pack, "first")

    def test_changed_answer_key_fails_fingerprint_verification(self):
        export_pack(self.pack, {"first": self.cases})
        (self.pack / "keys/first.json").write_text('{"opening":{"color":"red"}}')
        with self.assertRaisesRegex(ValueError, "fingerprint"):
            load_pack(self.pack, "first")

    def test_failed_export_leaves_no_partial_destination(self):
        self.original.unlink()
        with self.assertRaises(FileNotFoundError):
            export_pack(self.pack, {"first": self.cases})
        self.assertEqual(list(self.directory.iterdir()), [])

    def test_export_refuses_existing_empty_destination(self):
        self.pack.mkdir()
        with self.assertRaises(FileExistsError):
            export_pack(self.pack, {"first": self.cases})
        self.assertEqual(list(self.pack.iterdir()), [])

    def test_case_insensitive_suite_names_cannot_overwrite_other_inputs(self):
        with self.assertRaisesRegex(ValueError, "case-insensitive"):
            export_pack(self.pack, {"First": self.cases, "first": self.cases})
        self.assertFalse(self.pack.exists())
