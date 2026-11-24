"""Exercise the real HTTP transport, saved progress, and declared-denominator scoring."""

import hashlib
import json
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

from benchmarks.leo_bench.cases import Case, Image, Question
from benchmarks.leo_bench.protocol import score
from benchmarks.leo_bench.report import compare
from benchmarks.leo_bench.run import run_suite


class RunnerTests(unittest.TestCase):
    def test_local_run_preserves_image_order_and_resumes_without_resending(self):
        seen = []

        class Server(BaseHTTPRequestHandler):
            def do_POST(self):
                payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                seen.append(payload)
                data = json.dumps({"model": payload["model"], "choices": [
                    {"message": {"content": "yes"}, "finish_reason": "stop"}]}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *args):
                pass

        with tempfile.TemporaryDirectory() as directory, HTTPServer(("127.0.0.1", 0), Server) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                root = Path(directory)
                data = b"\xff\xd8frame"
                image = root / "frame.jpg"
                image.write_bytes(data)
                case = Case("sample", "Is the light on?", (Image(image, hashlib.sha256(data).hexdigest(), "0s"),),
                            (Question("light", ("yes", "no"), "yes"),))
                args = (root, "light", [case], "vision", f"http://127.0.0.1:{server.server_port}/v1", "", 32, 3, False)
                path = run_suite(*args)
                run_suite(*args)
                self.assertEqual(len(seen), 1)
                parts = seen[0]["messages"][0]["content"]
                self.assertEqual([part["type"] for part in parts], ["text", "image_url", "text"])
                self.assertEqual(parts[0]["text"], "0s")
                self.assertNotIn("expected", str(seen))
                self.assertIn("| 1/1 | 1 | 0 | 0 | 0 |", compare(root))
                saved = json.loads(path.read_text())
                saved["cases"]["sample"]["key"]["light"] = "no"
                path.write_text(json.dumps(saved))
                with self.assertRaisesRegex(ValueError, "settings changed"):
                    run_suite(*args)
            finally:
                server.shutdown()
                thread.join()

    def test_strict_answers_require_complete_unique_labels_and_natural_stop(self):
        case = Case("two", "questions", (), (Question("a", ("Y", "N"), "Y"), Question("b", ("Y", "N"), "N")))
        for reply, finish in [('{"a":"Y","a":"N","b":"N"}', "stop"), ('{"a":"Y"}', "stop"),
                              ('{"a":"Y","b":"X"}', "stop"), ('{"a":"Y","b":"N"}', "length")]:
            with self.subTest(reply=reply, finish=finish), self.assertRaises(ValueError):
                score(case, reply, finish)
