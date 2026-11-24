"""Send cases to one model and checkpoint each answer in a resumable local result file."""

import fcntl
import hashlib
import json
import os
import re
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener

from .cases import Case, fingerprint
from .protocol import messages, score


class NoRedirect(HTTPRedirectHandler):
    """Inputs and optional credentials belong only to the selected endpoint."""

    def redirect_request(self, *args):
        return None


def save(path: Path, data: dict):
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".checkpoint-", delete=False) as f:
        temporary = Path(f.name)
        try:
            f.write(json.dumps(data, indent=2, ensure_ascii=False).encode())
            f.flush()
            os.fsync(f.fileno())
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def request_case(endpoint: str, model: str, case: Case, tokens: int, timeout: int) -> dict:
    payload = {"model": model, "messages": messages(case), "temperature": 0,
               "max_tokens": tokens, "stream": False}
    headers = {"Content-Type": "application/json"}
    if key := os.environ.get("LEO_BENCH_API_KEY"):
        headers["Authorization"] = f"Bearer {key}"
    request = Request(endpoint + "/chat/completions", json.dumps(payload).encode(), headers)
    opener = build_opener(ProxyHandler({}), NoRedirect())
    with opener.open(request, timeout=timeout) as response:
        return json.load(response)


def protocol_fingerprint(cases: list[Case], tokens: int) -> str:
    """Bind ordered images, captions, full message layout, labels and generation settings."""
    protocol = [2, fingerprint(cases), [messages(c, hashes_only=True) for c in cases], tokens, 0]
    return hashlib.sha256(json.dumps(protocol).encode()).hexdigest()


def run_suite(directory: Path, suite: str, cases: list[Case], model: str, endpoint: str,
              revision: str, tokens: int, timeout: int, retry_errors: bool) -> Path:
    """Serialize writers, reject protocol drift on resume, and retain every observed attempt."""
    endpoint = endpoint.rstrip("/")
    url = urlsplit(endpoint)
    if url.scheme not in ("http", "https") or not url.hostname or url.username or url.password or url.query or url.fragment:
        raise ValueError("Expected an HTTP(S) base URL with no embedded credentials, query or fragment")
    identity = hashlib.sha256(json.dumps([model, revision]).encode()).hexdigest()[:12]
    slug = re.sub(r"[^A-Za-z0-9._-]+", "-", model)[:65] or "model"
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / f"{suite}--{slug}-{identity}.json"
    protocol_sha = protocol_fingerprint(cases, tokens)
    settings = dict(suite=suite, model=model, revision=revision, endpoint=endpoint,
                    protocol_sha256=protocol_sha, max_tokens=tokens, temperature=0, protocol_version=2)
    with path.with_suffix(".lock").open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise ValueError(f"Another process is running {path.name}") from exc
        keys = {c.id: {q.id: q.expected for q in c.questions} for c in cases}
        if path.exists():
            result = json.loads(path.read_text())
            if result["settings"] != settings or {k: v["key"] for k, v in result["cases"].items()} != keys:
                raise ValueError(f"Run settings changed. Use a fresh output directory: {path}")
        else:
            result = {"settings": settings, "created_utc": datetime.now(timezone.utc).isoformat(),
                      "cases": {k: {"key": key, "attempts": []} for k, key in keys.items()}}
            save(path, result)
        for number, case in enumerate(cases, 1):
            attempts = result["cases"][case.id]["attempts"]
            if attempts and not (retry_errors and attempts[-1]["status"] == "error"):
                continue
            start = time.monotonic()
            attempt = {"at": datetime.now(timezone.utc).isoformat(), "timeout_seconds": timeout, "status": "error"}
            stop = False
            try:
                reply = request_case(endpoint, model, case, tokens, timeout)
                attempt["response"] = reply
                choice = reply["choices"][0]
                reported = reply.get("model")
                if reported != model:
                    raise ValueError(f"Expected served model {model!r}, received {reported!r}")
                attempt["status"] = "invalid"
                attempt["answers"] = score(case, choice["message"]["content"], choice["finish_reason"])
                attempt["status"] = "scored"
            except HTTPError as exc:
                attempt["error"] = f"HTTP {exc.code}: {exc.read(2048).decode(errors='replace')}"
                stop = exc.code not in (400, 413, 422)
            except (ValueError, KeyError, TypeError, IndexError, AttributeError, OSError) as exc:
                attempt["error"] = str(exc)
                stop = attempt["status"] == "error"
            attempt["seconds"] = round(time.monotonic() - start, 3)
            attempts.append(attempt)
            save(path, result)
            print(f"{suite} {model} [{number}/{len(cases)}] {case.id} {attempt['status']} "
                  f"({attempt['seconds']}s)", flush=True)
            if stop:
                raise ValueError(f"Request failed: {attempt['error']}. Checkpoint: {path}")
    return path
