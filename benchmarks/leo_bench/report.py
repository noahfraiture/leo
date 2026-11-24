"""Read checkpointed model results without contacting a provider."""

import json
from pathlib import Path


def _assessed_labels(result: dict) -> dict:
    return {
        (case_id, question_id): attempt["answers"][question_id] == expected
        for case_id, row in result["cases"].items()
        for attempt in row["attempts"][-1:]
        if attempt["status"] == "scored"
        for question_id, expected in row["key"].items()
    }


def _paired(results: list[dict], baseline: str) -> list[str]:
    matches = [r for r in results if r["settings"]["model"] == baseline]
    if not matches:
        return [f"Awaiting baseline `{baseline}` for this suite and protocol.", ""]
    if len(matches) > 1:
        raise ValueError("Multiple revisions of the baseline. Compare one revision in a directory at a time.")
    reference = _assessed_labels(matches[0])
    lines = []
    for other in results:
        if other is matches[0]:
            continue
        labels = _assessed_labels(other)
        common = labels.keys() & reference.keys()
        gains = sum(labels[k] and not reference[k] for k in common)
        losses = sum(reference[k] and not labels[k] for k in common)
        lines.append(f"{other['settings']['model']} vs {baseline} on {len(common)} jointly assessed judgments: "
                     f"{gains} gained, {losses} lost, {len(common) - gains - losses} tied in correctness.")
    return lines


def compare(directory: Path, baseline: str | None = None) -> str:
    groups = {}
    for path in sorted(directory.glob("*.json")):
        result = json.loads(path.read_text())
        if not isinstance(result, dict) or "settings" not in result or "cases" not in result:
            continue
        settings = result["settings"]
        key = settings["suite"], settings["protocol_sha256"]
        groups.setdefault(key, []).append(result)
    if not groups:
        raise ValueError(f"No model benchmark results found in {directory}")
    lines = []
    for (suite, digest), results in sorted(groups.items()):
        lines.extend([f"## {suite} ({digest[:12]})", "",
                      "| Model | Correct / declared | Assessed | Invalid | Error | Pending | Time |",
                      "| --- | ---: | ---: | ---: | ---: | ---: | ---: |"])
        for result in results:
            total = correct = assessed = invalid = failed = pending = 0
            seconds = 0.0
            for row in result["cases"].values():
                n = len(row["key"])
                total += n
                seconds += sum(a["seconds"] for a in row["attempts"])
                if not row["attempts"]:
                    pending += n
                    continue
                attempt = row["attempts"][-1]
                if attempt["status"] == "scored":
                    assessed += n
                    correct += sum(attempt["answers"][k] == v for k, v in row["key"].items())
                elif attempt["status"] == "invalid":
                    invalid += n
                else:
                    failed += n
            settings = result["settings"]
            label = settings["model"] + (f" ({settings['revision']})" if settings["revision"] else "")
            lines.append(f"| {label} | {correct}/{total} | {assessed} | {invalid} | {failed} | {pending} | {seconds:.1f}s |")
        lines.append("")
        if baseline:
            lines.extend(_paired(results, baseline))
            lines.append("")
    return "\n".join(lines)
