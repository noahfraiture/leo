"""Export and reopen portable cases, private scoring keys and content-addressed images."""

import ctypes
import json
import os
from pathlib import Path
import re
import sys
from tempfile import TemporaryDirectory

from .cases import Case, Image, Question, fingerprint, image_bytes


def _manifest(directory: Path) -> dict[str, str]:
    document = json.loads(_member(directory.resolve(), Path("manifest.json")).read_bytes())
    if document.get("version") != 1 or not isinstance(document.get("suites"), dict) or not document["suites"]:
        raise ValueError("Expected a version 1 benchmark pack with at least one suite")
    suites = document["suites"]
    if len({name.casefold() for name in suites}) != len(suites):
        raise ValueError("Benchmark suite names must be distinct on case-insensitive filesystems")
    for name, digest in suites.items():
        if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_-]*", name) or not re.fullmatch(r"[a-f0-9]{64}", digest):
            raise ValueError(f"Invalid suite entry in benchmark pack: {name}")
    return suites


def pack_names(directory: Path) -> list[str]:
    """List suite identifiers without loading any images."""
    return list(_manifest(directory))


def _member(directory: Path, relative: Path) -> Path:
    path = directory / relative
    if not path.resolve().is_relative_to(directory):
        raise ValueError(f"Pack member points outside its directory: {relative}")
    return path


def load_pack(directory: Path, suite: str) -> list[Case]:
    """Verify the suite fingerprint and each distinct image before returning the cases."""
    directory = directory.resolve()
    suites = _manifest(directory)
    if suite not in suites:
        raise ValueError(f"Suite {suite!r} is absent from {directory}")
    inputs = json.loads(_member(directory, Path("inputs", suite + ".json")).read_bytes())
    keys = json.loads(_member(directory, Path("keys", suite + ".json")).read_bytes())
    cases = []
    verified = set()
    for entry in inputs:
        case_id = entry["id"]
        answers = keys.pop(case_id)
        questions = tuple(Question(q["id"], tuple(q["choices"]), answers.pop(q["id"])) for q in entry["questions"])
        if answers:
            raise ValueError(f"Unexpected scoring keys in {case_id}")
        images = []
        for frame in entry["images"]:
            digest = frame["sha256"]
            if not re.fullmatch(r"[a-f0-9]{64}", digest):
                raise ValueError(f"Invalid image digest in {case_id}")
            path = _member(directory, Path("images", digest))
            image = Image(path, digest, frame["caption"])
            if digest not in verified:
                image_bytes(image)
                verified.add(digest)
            images.append(image)
        cases.append(Case(case_id, entry["prompt"], tuple(images), questions))
    if keys or fingerprint(cases) != suites[suite]:
        raise ValueError(f"Pack contents differ from the registered fingerprint for {suite}")
    return cases


def _publish(staged: Path, destination: Path):
    # os.rename replaces an empty destination directory on POSIX. Use the native
    # exclusive rename to cover a competing writer creating it during the export.
    libc = ctypes.CDLL(None, use_errno=True)
    src, dst = os.fsencode(staged), os.fsencode(destination)
    if sys.platform == "darwin":
        result = libc.renamex_np(src, dst, 4)  # RENAME_EXCL
    elif sys.platform.startswith("linux"):
        result = libc.renameat2(-100, src, -100, dst, 1)  # AT_FDCWD, RENAME_NOREPLACE
    else:
        raise OSError("Atomic benchmark pack publication requires macOS or Linux")
    if result:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), str(destination))


def export_pack(destination: Path, suites: dict[str, list[Case]]):
    """Publish a verified, self-contained pack at a new destination."""
    if os.path.lexists(destination):
        raise FileExistsError(f"Benchmark pack destination already exists: {destination}")
    registered = {name: fingerprint(cases) for name, cases in suites.items()}
    destination.parent.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(dir=destination.parent, prefix=".benchmark-export-") as temporary:
        staged = Path(temporary) / "pack"
        staged.mkdir(mode=0o700)
        manifest = {"version": 1, "suites": registered}
        (staged / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        _manifest(staged)  # Check file-safe suite names before using them as paths.
        for name in ("inputs", "keys", "images"):
            (staged / name).mkdir()
        verified_sources = set()
        for name, cases in suites.items():
            inputs, keys = [], {}
            for case in cases:
                frames = []
                for image in case.images:
                    if not re.fullmatch(r"[a-f0-9]{64}", image.sha256):
                        raise ValueError(f"Invalid image digest in {case.id}")
                    target = staged / "images" / image.sha256
                    source = (image.path, image.sha256)
                    if source not in verified_sources:
                        data = image_bytes(image)
                        if not target.exists():
                            target.write_bytes(data)
                        verified_sources.add(source)
                    frames.append({"sha256": image.sha256, "caption": image.caption})
                inputs.append({
                    "id": case.id, "prompt": case.prompt, "images": frames,
                    "questions": [{"id": q.id, "choices": q.choices} for q in case.questions],
                })
                keys[case.id] = {q.id: q.expected for q in case.questions}
            for folder, data in (("inputs", inputs), ("keys", keys)):
                path = staged / folder / (name + ".json")
                path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            load_pack(staged, name)
        _publish(staged, destination)
