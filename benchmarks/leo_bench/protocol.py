"""The same ordered image inputs, answer contract and exact-label scorer for every model."""

import base64
import json

from .cases import Case, image_bytes


def answer_contract(case: Case) -> str:
    if len(case.questions) == 1:
        labels = ", ".join(case.questions[0].choices)
        return f"Return only one of these exact answers: {labels}."
    keys = "\n".join(f"{q.id}: {' | '.join(q.choices)}" for q in case.questions)
    return f"Return only one JSON object mapping each ID to one of its allowed answers.\n{keys}"


def _data_url(frame) -> str:
    data = image_bytes(frame)
    if data.startswith(b"\xff\xd8"):
        mime = "image/jpeg"
    elif data.startswith(b"\x89PNG\r\n\x1a\n"):
        mime = "image/png"
    else:
        raise ValueError(f"Expected JPEG or PNG: {frame.path}")
    return f"data:{mime};base64,{base64.b64encode(data).decode('ascii')}"


def messages(case: Case, *, hashes_only: bool = False) -> list[dict]:
    """Render the wire layout; use content hashes in place of image bytes for provenance."""
    content = []
    for frame in case.images:
        content.append({"type": "text", "text": frame.caption})
        url = f"sha256:{frame.sha256}" if hashes_only else _data_url(frame)
        content.append({"type": "image_url", "image_url": {"url": url}})
    content.append({"type": "text", "text": case.prompt + "\n\n" + answer_contract(case)})
    return [{"role": "user", "content": content}]


def score(case: Case, reply: str, finish: str) -> dict:
    """A malformed, truncated or incomplete response leaves the whole packet unassessed."""
    if finish != "stop":
        raise ValueError(f"Incomplete generation: {finish}")
    if len(case.questions) == 1:
        answers = {case.questions[0].id: reply.strip()}
    else:
        pairs = json.loads(reply, object_pairs_hook=list)
        if not isinstance(pairs, list) or not reply.lstrip().startswith("{"):
            raise ValueError("Expected an answer object")
        if any(not isinstance(pair, tuple) or not isinstance(pair[1], str) for pair in pairs):
            raise ValueError("Expected string answers")
        answers = dict(pairs)
        if len(answers) != len(pairs):
            raise ValueError("Duplicate answer ID")
    if answers.keys() != {q.id for q in case.questions}:
        raise ValueError("Missing or extra answer IDs")
    for q in case.questions:
        if answers[q.id] not in q.choices:
            raise ValueError(f"Unknown answer for {q.id}")
    return answers
