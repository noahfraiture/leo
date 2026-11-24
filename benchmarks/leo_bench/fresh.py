"""The archived fresh12 regression cases, including their evidence-removal controls."""

import hashlib
import json
from pathlib import Path

from .cases import Case, Image, Question

CAMPAIGN = "weight-space-2026-11-02"


def load(root: Path) -> list[Case]:
    directory = root / CAMPAIGN
    files = [
        ("benchmark-audit/new-source-sealed/transfer-packets.json", "6064d171d669514b6952a70928ee94e63dee2cfd28dbcd51fc8cf9cb16729eb9"),
        ("evaluator/adjudicated-transfer-gold.json", "3f975f7a6a64145d6d45c953abf27b597f1c472123b67cf008ca82ae8d188a6b"),
    ]
    documents = []
    for name, digest in files:
        data = (directory / name).read_bytes()
        if hashlib.sha256(data).hexdigest() != digest:
            raise ValueError(f"{name}: archive revision differs")
        documents.append(json.loads(data))
    packets, gold = documents
    labels = {(r["packet_id"], r["criterion_id"]): r["visible_input_label"] for r in gold["rows"] if r["scorable"]}
    cases = []
    for packet in packets["packets"]:
        frames = []
        for f in packet["frames"]:
            parts = Path(f["path"]).parts
            relative = Path(*parts[parts.index(CAMPAIGN) + 1:])
            frames.append(Image(directory / relative, f["sha256"], f'{f["seconds"]} seconds'))
        criteria = packet["criteria"]
        prompt = ("Judge each criterion from the supplied frames in order. Use met for visible fulfillment, "
                  "not_met for a visible contradiction, and cannot_verify for insufficient evidence.\n")
        prompt += "\n".join(f'{c["id"]}: {c["text"]}' for c in criteria)
        questions = tuple(Question(c["id"], ("met", "not_met", "cannot_verify"),
                                   labels.pop((packet["id"], c["id"]))) for c in criteria)
        cases.append(Case(packet["id"], prompt, tuple(frames), questions))
    if labels:
        raise ValueError("Unmatched fresh12 answer keys")
    return cases
