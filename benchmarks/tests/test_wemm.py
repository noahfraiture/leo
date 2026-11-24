"""Exercise the loopback bridge using Tencent's newline-delimited native protocol."""

import base64
import json
import os
from pathlib import Path
import select
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest
from urllib.error import HTTPError
from urllib.request import Request, urlopen


STUB = """#!/usr/bin/env python3
import json, os, sys, time
from pathlib import Path
Path(__file__).with_suffix('.pid').write_text(str(os.getpid()))
print('READY', flush=True)
for line in sys.stdin:
    request = json.loads((Path(line.strip()) / 'request.json').read_text())
    if 'hang-test' in request['prompt']:
        time.sleep(60)
    images = [Path(path).read_bytes().hex() for path in request['media']]
    result = {'text': json.dumps([request['prompt'], images, request['max_new_tokens']]),
              'finish_reason': 'length', 'input_tokens': 12, 'token_ids': [3],
              'native_config': {}, 'prefill_seconds': 0.01, 'seconds': 0.01, 'runtime': 'stub'}
    if 'native_prepared_media' in request:
        frames = request['native_prepared_media']
        assert all(f['video_frame'] for f in frames)
        assert [f['path'] for f in frames] == request['media']
        chunks = [{'media_id': f['id'], 'tokens': 2} for f in frames[::2]]
        result.update(input_protocol=request['native_input_protocol'], chunks=chunks,
                      input_token_ids=[248057] * (2 * len(chunks)))
        if 'unmerged-test' in request['prompt']:
            result['chunks'] = [{'media_id': f['id'], 'tokens': 1} for f in frames]
    print(json.dumps(result), flush=True)
"""


class WemmServerTest(unittest.TestCase):
    def setUp(self):
        temporary = TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        directory = Path(temporary.name)
        self.engine = directory / "wemm-native"
        self.engine.write_text(STUB)
        self.engine.chmod(0o700)
        for name in ("mmproj.gguf", "language.gguf"):
            (directory / name).touch()
        self.bridge = subprocess.Popen(
            [sys.executable, "-m", "benchmarks.providers.wemm", "--engine", str(self.engine),
             "--weights", str(directory), "--model", "fixture", "--paired-video-model", "fixture-video",
             "--port", "0", "--reply-timeout", "1"],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
        )
        self.addCleanup(self.stop_provider)
        self.assertTrue(select.select([self.bridge.stdout], [], [], 10)[0], "Provider startup timed out")
        ready = self.bridge.stdout.readline().split()
        self.assertEqual(ready[:2], ["Ready", "at"])
        self.address = ready[2]

    def stop_provider(self):
        self.bridge.terminate()
        self.bridge.wait(timeout=10)
        self.bridge.stdout.close()

    def request(self, content, model="fixture"):
        body = dict(model=model, messages=[{"role": "user", "content": content}],
                    max_tokens=23, stream=False, temperature=0)
        req = Request(self.address + "/chat/completions", data=json.dumps(body).encode(),
                      headers={"Content-Type": "application/json"})
        with urlopen(req, timeout=10) as response:
            return json.load(response)

    def test_http_request_preserves_interleaved_frames_and_native_finish(self):
        content = []
        for label, mime, data in (("start", "jpeg", b"image-one"), ("end", "png", b"image-two")):
            encoded = base64.b64encode(data).decode()
            content.extend([{"type": "text", "text": label},
                            {"type": "image_url", "image_url": {"url": f"data:image/{mime};base64,{encoded}"}}])
        content.append({"type": "text", "text": "Which changed?"})
        result = self.request(content)
        self.assertEqual(result["model"], "fixture")
        choice = result["choices"][0]
        self.assertEqual(choice["finish_reason"], "length")
        prompt, images, tokens = json.loads(choice["message"]["content"])
        self.assertIn("start\n<__media__>\nend\n<__media__>\nWhich changed?", prompt)
        self.assertEqual(images, [b"image-one".hex(), b"image-two".hex()])
        self.assertEqual(tokens, 23)

    def test_timeout_shuts_down_the_stalled_engine(self):
        with self.assertRaises(HTTPError) as error:
            self.request([{"type": "text", "text": "hang-test"}])
        self.assertEqual(error.exception.code, 503)
        error.exception.close()
        self.bridge.wait(timeout=10)
        with self.assertRaises(ProcessLookupError):
            os.kill(int(self.engine.with_suffix(".pid").read_text()), 0)

    def test_video_arm_places_both_captions_before_each_merged_pair(self):
        content = []
        for i in range(4):
            content.extend([{"type": "text", "text": f"frame {i}"},
                            {"type": "image_url", "image_url": {"url": "data:image/png;base64,YWJj"}}])
        content.append({"type": "text", "text": "Which changed?"})
        response = self.request(content, model="fixture-video")
        self.assertEqual(response["model"], "fixture-video")
        chunks = response["native"]["visual_chunks"]
        self.assertEqual([chunk["media_id"] for chunk in chunks], ["hf-video+0", "hf-video+2"])
        prompt, _, _ = json.loads(response["choices"][0]["message"]["content"])
        self.assertIn("frame 0\nframe 1\n<__media__><__media__>\nframe 2\nframe 3\n<__media__><__media__>", prompt)

    def test_supplemental_arm_rejects_an_unmerged_native_receipt(self):
        image = {"type": "image_url", "image_url": {"url": "data:image/png;base64,YWJj"}}
        parts = [{"type": "text", "text": "start"}, image, {"type": "text", "text": "end"}, image,
                 {"type": "text", "text": "unmerged-test"}]
        with self.assertRaises(HTTPError) as error:
            self.request(parts, model="fixture-video")
        self.assertEqual(error.exception.code, 503)
        error.exception.close()
