import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from benchmarks.leo_bench.checklist import _images


class ChecklistFramesTest(unittest.TestCase):
    def test_uses_only_initial_frames_in_sampling_order(self):
        # The historical cache also contains evidence selected after seeing model answers.
        with tempfile.TemporaryDirectory() as directory:
            analysis = Path(directory)
            cache = analysis / "cache"
            cache.mkdir()
            for index, offset in enumerate((2000, 1500, 0)):
                data = f"frame at {offset}".encode()
                (cache / f"{index}.jpg").write_bytes(data)
                (cache / f"{index}.json").write_text(json.dumps({
                    "candidateId": f"C1-T{offset}", "sha256": hashlib.sha256(data).hexdigest(),
                    "sizeBytes": len(data),
                }))
            evidence = {"frames": [
                {"id": f"C1-T{ms}", "cameraId": 1, "sessionOffsetMs": ms} for ms in (0, 2000)
            ]}
            result = _images(analysis, evidence)
            self.assertEqual([image.path.name for image in result], ["2.jpg", "0.jpg"])
