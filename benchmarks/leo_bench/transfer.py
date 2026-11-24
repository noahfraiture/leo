"""Replay the frozen 16-frame Perception Test questions and packet-specific labels."""

import hashlib
import json
from functools import partial
from pathlib import Path

from .cases import Case, Image, Question


CAMPAIGN = "video-transfer-2026-11-04"
GOLD = {
    "donor": (
        "gold-root-v1/DONOR-GOLD-V2.json",
        "989e52c860dbea6e06d6a09c323ec93c5e6959e64f3dbc6503acc37b26c19373",
    ),
    "development": (
        "gold-root-v1/DEVELOPMENT-GOLD-V1.json",
        "35e4fd16343ef322738593ca15c3bc598339e7b40f6644886ecb19b251dddb32",
    ),
    "confirmation": (
        "confirmation-gold-adjudication-v1/PRIMARY-GOLD-96.json",
        "f851104e8965b3f292821d007dc29d3b847da0b55af0b6d613b3192d5e299ce5",
    ),
}
SCENES = (
    "gold-root-v1/SCENE-GROUPS-FINAL-V3.json",
    "9ee1156d140420d9df1952443ad49df454f9545a43eaf409cb20afba289c6eb1",
)


def _frozen_json(path: Path, expected_digest: str) -> dict:
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != expected_digest:
        raise ValueError(f"Archived document has changed: {path}")
    return json.loads(data)


def _case(packet_dir: Path, gold: dict) -> Case:
    packet = _frozen_json(packet_dir / "packet-16.json", gold["packet_sha256"])
    question_id = gold["question_id"]
    questions = [q for q in packet["questions"] if q["id"] == question_id]
    if len(questions) != 1 or packet["video_id"] != gold["video_id"] or len(packet["frames"]) != 16:
        raise ValueError(f"Question does not match its registered 16-frame packet: {question_id}")
    question = questions[0]
    options = question["options"]
    choices = tuple(option["label"] for option in options)
    labels = gold["allowed_answer_labels"]
    if len(labels) != 1 or labels[0] not in choices:
        raise ValueError(f"Missing a unique reference option for {question_id}")
    lines = [f"{option['label']}. {option['text']}" for option in options]
    prompt = "\n".join([
        "Answer from the supplied chronological images. If the evidence is insufficient, "
        f"choose {question['abstention_display_label']}.",
        f"{question_id}  {question['question']}",
        *lines,
    ])
    # Frozen paths are absolute on the original machine. Their filenames and byte
    # digests identify the same images inside a relocated copy of the archive.
    images = tuple(
        Image(packet_dir / Path(frame["path"]).name, frame["sha256"],
              f"{frame['frame_id']} at {frame['pts_seconds']} s")
        for frame in packet["frames"]
    )
    return Case(question_id, prompt, images, (Question(question_id, choices, labels[0]),))


def load(root: Path, split: str = "confirmation") -> list[Case]:
    """Load one archive split. Linked confirmation scenes form a separate suite."""
    directory = root / CAMPAIGN / "benchmark"
    role = "confirmation" if split == "linked" else split
    gold_path, digest = GOLD[role]
    rows = _frozen_json(directory / gold_path, digest)["rows"]
    scenes = _frozen_json(directory / SCENES[0], SCENES[1])["groups"]
    cases = []
    for row in rows:
        # The second frame count is a paired diagnostic on these same questions.
        if row["frame_count"] != 16:
            continue
        video = row["video_id"]
        scene = scenes[video]
        if scene["role"] != role:
            raise ValueError(f"Source role mismatch for {video}")
        if role == "confirmation":
            linked_scene = not scene["confirmation_unseen_scene_eligible"]
            if linked_scene != (split == "linked"):
                continue
        cases.append(_case(directory / "frames-v1" / role / video, row))
    return cases


load_development = partial(load, split="development")
load_confirmation = partial(load, split="confirmation")
load_linked = partial(load, split="linked")
load_donor = partial(load, split="donor")
