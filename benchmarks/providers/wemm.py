"""Serve the benchmark's ordered images and prompts through Tencent's native WeMM engine."""

import argparse
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import signal
import subprocess
from tempfile import TemporaryDirectory


def native_request(request: dict, folder: Path, *, paired_video: bool = False) -> dict:
    """Render Tencent's chat template and materialize the images for one inference."""
    messages = request["messages"]
    if len(messages) != 1 or messages[0]["role"] != "user":
        raise ValueError("Expected one user message")
    if request["temperature"] != 0 or request.get("stream", False):
        raise ValueError("Expected a non-streaming request with temperature zero")
    tokens = request["max_tokens"]
    if type(tokens) is not int or tokens < 1:
        raise ValueError("Expected a positive integer max_tokens")

    media, lines, digests = [], [], []
    parts = messages[0]["content"]
    for part in parts:
        if part["type"] == "text":
            lines.append(part["text"])
        elif part["type"] == "image_url":
            mime, encoded = part["image_url"]["url"].split(",", 1)
            extension = {"data:image/jpeg;base64": "jpg", "data:image/png;base64": "png"}.get(mime)
            if not extension:
                raise ValueError("Expected an inline JPEG or PNG image")
            path = folder / f"frame-{len(media):03d}.{extension}"
            data = base64.b64decode(encoded, validate=True)
            path.write_bytes(data)
            if paired_video:
                digests.append(hashlib.sha256(data).hexdigest())
            media.append(str(path))
            lines.append("<__media__>")
        else:
            raise ValueError(f"Unsupported input type: {part['type']}")
    prepared = {"media": media, "max_new_tokens": tokens, "fps": 2, "save_embeddings": False}
    if paired_video:
        kinds = [part["type"] for part in parts]
        if not media or len(media) % 2 or kinds != ["text", "image_url"] * len(media) + ["text"]:
            raise ValueError("Paired video expects an even number of captioned images followed by one task")
        # mtmd merges only adjacent media markers. Keep both exact captions,
        # then emit both frame markers with no intervening whitespace.
        paired_lines = []
        for i in range(0, len(media), 2):
            paired_lines.extend([lines[2 * i], lines[2 * i + 2], "<__media__><__media__>"])
        lines = [*paired_lines, lines[-1]]
        prepared["native_input_protocol"] = "hf_uniform_video_v1"
        prepared["native_prepared_media"] = [
            {"path": path, "id": f"hf-video+{i}", "video_frame": True} for i, path in enumerate(media)
        ]
        prepared["native_video_metadata"] = {"ordered_frame_sha256": digests,
                                             "timestamp_placement": "both original captions before each frame pair"}

    prompt = "<|im_start|>system\nYou are a helpful assistant.<|im_end|>\n<|im_start|>user\n"
    # The native launcher uses this no-thinking prefix for GGUF generation.
    prompt += "\n".join(lines) + "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"
    prepared["prompt"] = prompt
    return prepared


def completion(engine: subprocess.Popen, folder: Path, model: str, timeout: int, prepared: dict) -> dict:
    """Submit one saved request via the engine's directory-per-line protocol."""
    engine.stdin.write(str(folder) + "\n")
    engine.stdin.flush()
    # This loopback server handles one request on its main thread. Stop a hung
    # engine so a late line can never be mistaken for the next case's response.
    signal.setitimer(signal.ITIMER_REAL, timeout)
    try:
        line = engine.stdout.readline()
    except TimeoutError:
        engine.kill()
        engine.wait()
        raise
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
    if not line:
        raise RuntimeError("Native model process exited")
    result = json.loads(line)
    if "error" in result:
        raise RuntimeError(result["error"])
    choice = {
        "message": {"role": "assistant", "content": result["text"]},
        "finish_reason": {"eos": "stop", "length": "length"}[result["finish_reason"]],
    }
    native = {k: result[k] for k in ("native_config", "prefill_seconds", "seconds", "runtime")}
    if "native_prepared_media" in prepared:
        # Each merged chunk in the pinned runtime carries its FIRST frame's ID.
        # Fail the request if any frame went through the ordinary still-image path.
        visual = [chunk for chunk in result["chunks"] if "media_id" in chunk]
        first_ids = [item["id"] for item in prepared["native_prepared_media"][::2]]
        if (result["input_protocol"] != "hf_uniform_video_v1" or [c["media_id"] for c in visual] != first_ids
                or result["input_token_ids"].count(248057) != sum(c["tokens"] for c in visual)):
            raise RuntimeError("Native engine did not confirm the registered temporal pairs")
        native.update(input_protocol=result["input_protocol"], visual_chunks=visual,
                      video_metadata=prepared["native_video_metadata"],
                      prompt_sha256=hashlib.sha256(prepared["prompt"].encode()).hexdigest())
    return {
        "model": model, "choices": [choice],
        "usage": {"prompt_tokens": result["input_tokens"], "completion_tokens": len(result["token_ids"])},
        "native": native,
    }


class Handler(BaseHTTPRequestHandler):
    """Translate the two loopback endpoints used by the benchmark runner."""

    def respond(self, status: int, body: dict):
        data = json.dumps(body, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/v1/models":
            self.respond(200, {"data": [{"id": name} for name in self.server.models]})
        else:
            self.respond(404, {"error": "Unknown route"})

    def do_POST(self):
        if self.path != "/v1/chat/completions":
            self.respond(404, {"error": "Unknown route"})
            return
        with TemporaryDirectory(prefix="wemm-input-") as temporary:
            folder = Path(temporary).resolve()
            try:
                length = int(self.headers["Content-Length"])
                if length < 1:
                    raise ValueError("Expected a request body")
                request = json.loads(self.rfile.read(length))
                model = request["model"]
                if model not in self.server.models:
                    raise ValueError("Unknown model")
                prepared = native_request(request, folder, paired_video=model == self.server.video_model)
            except (AttributeError, KeyError, IndexError, TypeError, ValueError) as error:
                self.respond(400, {"error": str(error)})
                return
            try:
                (folder / "request.json").write_text(json.dumps(prepared), encoding="utf-8")
                response = completion(self.server.engine, folder, model, self.server.reply_timeout, prepared)
            except (AttributeError, KeyError, IndexError, TypeError, ValueError, RuntimeError, OSError) as error:
                self.respond(503, {"error": str(error)})
                return
            self.respond(200, response)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True, help="Path to the compiled wemm-native binary")
    parser.add_argument("--weights", type=Path, required=True, help="Directory with language.gguf and mmproj.gguf")
    parser.add_argument("--model", required=True, help="Model ID to expose via /v1/models")
    parser.add_argument("--paired-video-model", help="Optional separate ID for native temporal frame pairs")
    parser.add_argument("--port", type=int, default=18137)
    parser.add_argument("--context", type=int, default=65536)
    parser.add_argument("--gpu-layers", type=int, default=99, help="Offloaded language layers (0 for CPU)")
    parser.add_argument("--reply-timeout", type=int, default=900, help="Maximum seconds per inference")
    args = parser.parse_args()
    if args.reply_timeout <= 0:
        parser.error("--reply-timeout must be positive")
    if args.paired_video_model == args.model:
        parser.error("The paired video arm needs a distinct model ID")
    for name in ("language.gguf", "mmproj.gguf"):
        if not (args.weights / name).is_file():
            parser.error(f"Missing weights file: {args.weights / name}")

    # SIGINT and SIGTERM both unwind the request and release the owned model.
    def stop(signum, frame):
        raise KeyboardInterrupt

    def inference_timeout(signum, frame):
        raise TimeoutError(f"Native inference exceeded {args.reply_timeout} seconds")

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGALRM, inference_timeout)
    command = [str(args.engine.resolve()), str(args.weights / "language.gguf"), str(args.weights / "mmproj.gguf"),
               str(args.gpu_layers), "6", str(args.context), "512", "4", "768", "2"]
    with HTTPServer(("127.0.0.1", args.port), Handler) as server:
        server.video_model = args.paired_video_model
        server.models = [args.model, args.paired_video_model] if args.paired_video_model else [args.model]
        server.reply_timeout = args.reply_timeout
        with subprocess.Popen(command, env=dict(os.environ, WEMM_NATIVE_VISION_FLASH_ATTN="0"),
                              stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
                              encoding="utf-8", start_new_session=True) as engine:
            server.engine = engine
            try:
                if engine.stdout.readline().strip() != "READY":
                    raise RuntimeError("Native model startup failed. See engine diagnostics above.")
                print(f"Ready at http://127.0.0.1:{server.server_port}/v1 for {args.model}", flush=True)
                while engine.poll() is None:
                    server.handle_request()
            except KeyboardInterrupt:
                pass
            finally:
                engine.terminate()
                try:
                    engine.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    engine.kill()


if __name__ == "__main__":
    main()
