"""Locate readable reports in Leo's on-disk evaluation archive."""

import re
from pathlib import Path


_OVERVIEW_NAMES = (
    "report.md", "report.txt", "handoff.md", "result.txt",
    "stop.md", "review.md", "state.md", "progress.md", "protocol.md",
    "candidate-summary.json", "baseline-summary.json",
)


def _report_rank(path: Path, in_report_folder: bool) -> tuple[int, int] | None:
    """Rank newer final reports first and keep alternate formats near the top."""
    name = path.name.lower()
    final = re.fullmatch(r"final[-_]report(?:[-_]v(\d+))?\.(?:md|txt|html)", name)
    if final:
        return (-3, -int(final[1] or 0))
    if name.endswith(".html") and ("report" in name or (in_report_folder and name == "index.html")):
        return (-2, 0)
    if name in _OVERVIEW_NAMES:
        return (_OVERVIEW_NAMES.index(name), 0)
    if name.endswith(".md") and (
        name.startswith(("report-20", "completion-checkpoint-"))
        or name.endswith(("pilot-report.md", "pilot_report.md"))
    ):
        return (-1, 0)
    return None


def _reports(campaign: Path) -> list[str]:
    # Keep one current document per kind. Several campaigns retain every draft
    # and revision alongside the published report.
    choices: dict[int, tuple[int, str]] = {}
    for folder in (campaign, campaign / "report", campaign / "reports"):
        if not folder.is_dir():
            continue
        for path in folder.iterdir():
            rank = _report_rank(path, in_report_folder=folder != campaign)
            if rank is None or not path.is_file():
                continue
            kind, revision = rank
            entry = (revision, path.relative_to(campaign.parent).as_posix())
            if kind not in choices or entry < choices[kind]:
                choices[kind] = entry
    return [choices[kind][1] for kind in sorted(choices)[:3]]


def catalog(root: Path) -> list[dict]:
    """List archive directories and their overview documents, relative to root.

    The archive includes diagnostics and retrospective reviews as well as model
    evaluations. A directory with no overview still appears with an empty list.
    """
    return [
        {"name": path.name, "reports": _reports(path)}
        for path in sorted(root.iterdir())
        if not path.name.startswith(".") and path.is_dir()
    ]
