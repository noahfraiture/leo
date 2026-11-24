"""Archived Leo checklist cases, scored against the matched-33 v27 source inventory.

The household clips reuse the simple-flow campaign's complete initial JPEG sets.
The two drone clips reuse the original matched-33 cache, before adaptive sampling.
Each case uses the archived checklist and corresponding initial sampling schedule.
"""

import hashlib
import json
from pathlib import Path
from uuid import UUID

from .cases import Case, Image, Question


INVENTORY = Path("evidence-index-2026-09-17/benchmark-source-inventory-v27-v1.json")
INVENTORY_SHA256 = "719c38430ce2630adab68b84c438169c475dab51feabc9ac00501d3fabba341c"
MATCHED = Path("local-reliability-night-2026-09-15/matched-33-v1")
SIMPLE = Path("simple-evidence-2026-09-20/trials")
CHOICES = ("met", "notMet", "cannotVerify")


def _images(analysis: Path, evidence: dict) -> tuple[Image, ...]:
    """Bind the initial sampling plan to archived JPEGs, ignoring later revisits."""
    planned = evidence["frames"]
    wanted = {frame["id"] for frame in planned}
    files = {}
    for sidecar in (analysis / "cache").glob("*.json"):
        metadata = json.loads(sidecar.read_bytes())
        frame_id = metadata["candidateId"]
        if frame_id not in wanted:
            continue
        if frame_id in files:
            raise ValueError(f"Duplicate archived frame {frame_id} in {analysis}")
        jpeg = sidecar.with_suffix(".jpg")
        if jpeg.stat().st_size != metadata["sizeBytes"]:
            raise ValueError(f"Archived JPEG length differs: {jpeg}")
        files[frame_id] = (jpeg, metadata)
    if set(files) != wanted or len(wanted) != len(planned):
        raise ValueError(f"Incomplete or ambiguous initial JPEG set in {analysis}")

    images = []
    for frame in planned:
        jpeg, metadata = files[frame["id"]]
        caption = f'Camera {frame["cameraId"]}, requested session offset {frame["sessionOffsetMs"]} ms'
        if "decodedRecordingOffsetUs" in metadata:
            caption += f', decoded recording offset {metadata["decodedRecordingOffsetUs"]} microseconds'
        images.append(Image(jpeg, metadata["sha256"], caption))
    return tuple(images)


def _cases(root: Path, numbers: tuple[str, ...], *, drone: bool) -> list[Case]:
    inventory_bytes = (root / INVENTORY).read_bytes()
    if hashlib.sha256(inventory_bytes).hexdigest() != INVENTORY_SHA256:
        raise ValueError("Matched-33 source inventory differs from the frozen v27 file")
    inventory = {case["case"]: case for case in json.loads(inventory_bytes)["cases"]}
    cases = []
    for number in numbers:
        if drone:
            campaign = root / MATCHED
            registration = campaign / "runs/current-selected" / number / "registration.json"
            session = campaign / "data/sessions" / f"case-{number}"
        else:
            trial = root / SIMPLE / f"simple-short{number}-v1"
            registration = trial / "run.json"
            session = trial / "session"
        run_id = UUID(json.loads(registration.read_bytes())["runId"])
        analysis = session / "analyses" / str(run_id)
        inputs = json.loads((analysis / "inputs.json").read_bytes())
        evidence = inputs["initialEvidence"]
        source = inventory[number]
        criteria = inputs["exercise"]["checklist"]["criteria"]
        if ([row["text"] for row in criteria] != [row["text"] for row in source["checks"]]
                or [row["sha256"] for row in evidence["sources"]] != [row["sha256"] for row in source["recordings"]]
                or evidence["eligiblePeriods"] != source["eligiblePeriods"]
                or len(evidence["frames"]) != source["initialFrames"]):
            raise ValueError(f"Case {number} differs from the archived checklist or media schedule")

        prompt = ["Judge each checklist item from the supplied chronological samples. "
                  "Use met for a visibly satisfied requirement, notMet when the evidence shows "
                  "it was not satisfied, and cannotVerify when the evidence is insufficient."]
        questions = []
        for check in source["checks"]:
            prompt.append(f'{check["originalId"]}: {check["text"]}')
            questions.append(Question(check["originalId"], CHOICES, check["expectedJudgment"]))
        cases.append(Case(f"matched-{number}", "\n".join(prompt), _images(analysis, evidence), tuple(questions)))
    return cases


def load(root: Path) -> list[Case]:
    """Load the five household recordings with 18 original checklist labels."""
    return _cases(root, ("06", "07", "08", "09", "10"), drone=False)


def load_drone(root: Path) -> list[Case]:
    """Load the two drone-handling recordings with eight original checklist labels."""
    return _cases(root, ("01", "02"), drone=True)
