"""Frozen, model-independent inputs for a scored vision benchmark."""

import hashlib
import json
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Image:
    path: Path
    sha256: str
    caption: str


@dataclass(frozen=True)
class Question:
    id: str
    choices: tuple[str, ...]
    expected: str


@dataclass(frozen=True)
class Case:
    id: str
    prompt: str
    images: tuple[Image, ...]
    questions: tuple[Question, ...]


def fingerprint(cases: list[Case]) -> str:
    """Validate the scoring keys and bind all visible inputs and labels, independent of location."""
    if not cases or len({case.id for case in cases}) != len(cases):
        raise ValueError("Suite has missing or duplicate cases")
    manifest = []
    for case in cases:
        if not case.prompt.strip() or not case.images or not case.questions:
            raise ValueError(f"{case.id}: missing prompt, images or questions")
        if len({q.id for q in case.questions}) != len(case.questions):
            raise ValueError(f"{case.id}: duplicate question ID")
        for q in case.questions:
            if not q.id or len(set(q.choices)) != len(q.choices) or len(q.choices) < 2 or q.expected not in q.choices:
                raise ValueError(f"{case.id}: invalid answer key for {q.id}")
        manifest.append([case.id, case.prompt, [(i.sha256, i.caption) for i in case.images],
                         [(q.id, q.choices, q.expected) for q in case.questions]])
    return hashlib.sha256(json.dumps(manifest, ensure_ascii=False).encode()).hexdigest()


def image_bytes(image: Image) -> bytes:
    data = image.path.read_bytes()
    if hashlib.sha256(data).hexdigest() != image.sha256:
        raise ValueError(f"Frame differs from archive manifest: {image.path}")
    return data
