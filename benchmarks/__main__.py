"""Command-line entry point for archived model comparisons."""

import argparse
import os
import sys
from functools import partial
from pathlib import Path

from .leo_bench import archive, checklist, fresh, transfer
from .leo_bench.cases import fingerprint, image_bytes
from .leo_bench.pack import export_pack, load_pack, pack_names
from .leo_bench.report import compare
from .leo_bench.run import run_suite

SUITES = {
    "leo-household": (checklist.load, "Five household recordings, original checklist"),
    "public-evidence": (fresh.load, "Sequences and evidence-removal controls on three public recordings"),
    "perception-confirmation": (transfer.load_confirmation, "Twenty public recordings with separately grouped scenes"),
    "perception-development": (transfer.load_development, "Twelve public development recordings"),
    "perception-donor": (transfer.load_donor, "Twelve public recordings used in method development"),
    "perception-linked": (transfer.load_linked, "Four confirmation recordings linked to development or donor scenes"),
    "leo-drone": (checklist.load_drone, "Two equipment-handling recordings, up to 56 images per request"),
}
DEFAULT = ["leo-household", "public-evidence", "perception-confirmation"]


def data_home() -> Path:
    if sys.platform == "darwin":
        return Path.home() / "Library/Application Support/Leo"
    return Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")) / "leo"


def positive(value: str) -> int:
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("Expected a positive integer")
    return number


def main():
    parser = argparse.ArgumentParser(prog="python3 -m benchmarks", description=__doc__)
    parser.add_argument("--archive", type=Path, default=os.environ.get("LEO_EVALUATIONS"),
                        help="Historical evaluations directory (or set LEO_EVALUATIONS)")
    parser.add_argument("--pack", type=Path, help="Use a portable benchmark pack instead of the historical archive")
    commands = parser.add_subparsers(dest="action", required=True)
    commands.add_parser("list", help="List reusable suites and their case counts")
    history = commands.add_parser("archive", help="Find overview documents for every historical campaign")
    history.add_argument("--markdown", action="store_true", help="Print a linked historical index")
    validate = commands.add_parser("validate", help="Check answer keys and original image bytes offline")
    run = commands.add_parser("run", help="Benchmark one or more served vision models")
    export = commands.add_parser("export", help="Create a movable benchmark pack from selected suites")
    export.add_argument("destination", type=Path, help="New directory for the self-contained pack")
    for cmd in [validate, run, export]:
        cmd.add_argument("--suite", action="append", help="Suite name or all. Repeat to select several suites")
    run.add_argument("--base-url", default="http://127.0.0.1:1234/v1")
    run.add_argument("--model", action="append", required=True, help="Exact served model ID (repeatable)")
    run.add_argument("--revision", default="", help="Optional checkpoint or runtime identifier for this model")
    run.add_argument("--out", type=Path, default=data_home() / "benchmark-runs" / "comparisons")
    run.add_argument("--max-tokens", type=positive, default=2048, help="Completion budget per request (default: 2048)")
    run.add_argument("--timeout", type=positive, default=900, help="Request timeout in seconds")
    run.add_argument("--retry-errors", action="store_true", help="Retry recorded request errors, preserving all attempts")
    report = commands.add_parser("compare", help="Print side-by-side scores from saved runs")
    report.add_argument("--baseline", help="Show paired gains and losses against this served model ID")
    report.add_argument("directory", type=Path, nargs="?", default=data_home() / "benchmark-runs" / "comparisons")
    args = parser.parse_args()
    if args.action == "compare":
        print(compare(args.directory, args.baseline))
        return
    root = (args.archive or data_home() / "evaluations").expanduser()
    explicit_pack = args.pack is not None
    installed_pack = data_home() / "benchmarks" / "vision-v1"
    if args.action in ("run", "validate", "list") and not args.archive and not explicit_pack and installed_pack.is_dir():
        args.pack = installed_pack
    if not args.pack and not root.is_dir():
        parser.error(f"Evaluation archive not found: {root}. Set LEO_EVALUATIONS or --archive.")
    if args.action == "archive":
        if args.pack:
            parser.error("The archive command needs the original campaigns. Use list to inspect a pack.")
        items = archive.catalog(root)
        if args.markdown:
            print("# Leo evaluation archive\n")
            print(f"{len(items)} campaign, review and diagnostic directories.\n")
        for item in items:
            if args.markdown:
                folder = (root / item["name"]).resolve()
                print(f"## [{item['name']}](<{folder}>)\n")
                for path in item["reports"]:
                    print(f"- [{Path(path).name}](<{(root / path).resolve()}>)")
                print()
            else:
                print(item["name"])
                for path in item["reports"]:
                    print(f"  {root / path}")
        return
    if args.pack:
        suites = {s: (partial(load_pack, suite=s), SUITES.get(s, (None, "Imported suite"))[1])
                  for s in pack_names(args.pack)}
        root = args.pack
    else:
        suites = SUITES
    if args.action == "list":
        for suite, (loader, description) in suites.items():
            try:
                cases = loader(root)
                counts = f"{len(cases)} requests, {sum(len(c.questions) for c in cases)} judgments"
            except (OSError, ValueError, KeyError) as exc:
                counts = f"unavailable: {exc}"
            print(f"{suite:24} {description} ({counts})")
        return
    default_suites = list(suites)
    if args.action == "run" and not explicit_pack and all(name in suites for name in DEFAULT):
        default_suites = DEFAULT
    selected = args.suite or default_suites
    if unknown := set(selected) - {*suites, "all"}:
        parser.error(f"Unknown suite(s): {', '.join(sorted(unknown))}. Use list to see the available suites.")
    if "all" in selected:
        selected = list(suites)
    sets = {}
    verified = set()
    for suite in dict.fromkeys(selected):
        cases = suites[suite][0](root)
        digest = fingerprint(cases)
        for case in cases:
            for image in case.images:
                key = image.path, image.sha256
                if key not in verified:
                    image_bytes(image)
                    verified.add(key)
        print(f"{suite}: {len(cases)} requests, {sum(len(c.questions) for c in cases)} judgments, {digest}", flush=True)
        sets[suite] = cases
    if args.action == "export":
        export_pack(args.destination, sets)
        print(f"Saved portable benchmark pack to {args.destination}")
    if args.action == "run":
        if args.revision and len(args.model) > 1:
            parser.error("Pass a revision with a single model")
        for model in dict.fromkeys(args.model):
            for suite, cases in sets.items():
                path = run_suite(args.out, suite, cases, model, args.base_url, args.revision,
                                 args.max_tokens, args.timeout, args.retry_errors)
                print(f"Saved {path}", flush=True)
        print(compare(args.out))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError) as exc:
        sys.exit(str(exc))
