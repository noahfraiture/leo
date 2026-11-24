"""Regression checks for relocating frozen Perception Test packets."""

import hashlib
import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from benchmarks.leo_bench.transfer import _case


class TransferPacketTests(unittest.TestCase):
    def test_packet_uses_its_own_option_order_and_relocated_frames(self):
        with TemporaryDirectory() as temporary:
            directory = Path(temporary)
            frames = [
                {"frame_id": f"f{i}", "pts_seconds": i, "path": f"/old/location/{i}.jpg", "sha256": "a" * 64}
                for i in range(16)
            ]
            packet = {
                "video_id": "clip",
                "frames": frames,
                "questions": [{
                    "id": "clip-q2", "question": "Is the light on?",
                    "abstention_display_label": "B",
                    "options": [{"label": "A", "text": "yes"}, {"label": "B", "text": "unclear"}],
                }],
            }
            data = json.dumps(packet).encode()
            (directory / "packet-16.json").write_bytes(data)
            gold = {
                "video_id": "clip", "question_id": "clip-q2",
                "packet_sha256": hashlib.sha256(data).hexdigest(), "allowed_answer_labels": ["A"],
                "explanation": "Private adjudication note",
            }

            case = _case(directory, gold)

            self.assertEqual(case.images[-1].path, directory / "15.jpg")
            self.assertIn("choose B", case.prompt)
            self.assertIn("A. yes\nB. unclear", case.prompt)
            self.assertNotIn("Private adjudication note", case.prompt)
            self.assertEqual(case.questions[0].expected, "A")
