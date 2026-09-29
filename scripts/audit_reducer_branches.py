"""Audit source branch outcomes in the Slice 0 reducer coverage export.

Rust 1.91.1 emits these counters with `-Z coverage-options=branch`. LLVM
exports repeated source locations when the library is present in both test
binaries, so each location is combined across binaries before auditing.
"""

from __future__ import annotations

import json
import sys
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "crates/runtime-core/src/lib.rs"
SOURCE_SUFFIX = "runtime-core/src/lib.rs"

# Each entry is (source line, missing counter side): (expected count, proof).
# These are defensive paths excluded by the structural invariant or by an
# earlier gate in the same transition. An unexpected missed direction fails CI.
KNOWN_UNREACHABLE = {
    ("|| state.active_session.is_none()", "first"): (1, "focused state has an active session"),
    ("&& state.active_session.is_some()", "second"): (1, "focused state has an active session"),
    ("if let Some(session) = state.active_session {", "second"): (1, "an Eat decision requires an active session"),
    ("&& state.focus == FocusState::Focused", "second"): (1, "matching active session implies focused state"),
    ("CompositionState::Active { epoch } if old_session.is_some() => {", "second"): (1, "active composition without Applying requires an active session"),
    ("} else if !matches!(state.composition, CompositionState::Terminating { .. }) {", "second"): (1, "Applying commit blocks composition effects that could start termination"),
    ("if state.active_session.is_none() && state.focus == FocusState::Unfocused {", "second"): (1, "no active session implies unfocused state"),
    ("pending.identity.request_seq == request_seq", "second"): (1, "deadline matches the only in-flight request; commit blocks later dispatch"),
    ("if let CommitStatus::Applying { effect_id } = pending.status {", "second"): (1, "pending commit is always Applying"),
    ("if let Some(index) = state.commits.iter().position(|entry| {", "second"): (1, "Applying commit is present in the terminal cache"),
    ("entry.identity == pending.identity && entry.commit_id == pending.commit_id", "second"): (2, "pending commit and matching cache entry are created together"),
    (") if expected == effect_id => {", "second"): (2, "matched outstanding composition effect owns the transient state"),
    ("} else if let Some(session) = scope_session {", "second"): (1, "composition effect has composition scope"),
    ("if let Some(pending) = state.pending_commit.filter(|pending| {", "second"): (1, "matched HostCommit effect has an Applying pending commit"),
    ("matches!(pending.status, CommitStatus::Applying { effect_id: id } if id == effect_id)", "second"): (1, "HostCommit expectation and pending effect ID agree"),
    ("if let Some(index) = state.commits.iter().position(|entry| entry.is_some_and(|entry| {", "second"): (1, "pending commit has a matching terminal cache entry"),
}


def audit(export_path: Path) -> int:
    source_lines = SOURCE.read_text(encoding="utf-8").splitlines()
    start = next(i for i, line in enumerate(source_lines, 1) if line.startswith("fn reduce_key_callback("))
    end = next(i for i, line in enumerate(source_lines, 1) if line.startswith("fn validate_effect_batch("))

    coverage = json.loads(export_path.read_text(encoding="utf-8-sig"))
    runtime_file = next(
        file
        for data in coverage["data"]
        for file in data["files"]
        if file["filename"].replace("\\", "/").endswith(SOURCE_SUFFIX)
    )
    by_location: dict[tuple[int, int, int, int], list[int]] = {}
    for branch in runtime_file["branches"]:
        line = branch[0]
        if not start <= line < end:
            continue
        key = tuple(branch[:4])
        counts = by_location.setdefault(key, [0, 0])
        counts[0] = max(counts[0], branch[4])
        counts[1] = max(counts[1], branch[5])

    if not by_location:
        raise ValueError("coverage export has no reducer branch locations")

    missing: Counter[tuple[str, str]] = Counter()
    for (line, _, _, _), counts in by_location.items():
        text = source_lines[line - 1].strip()
        for side, count in zip(("first", "second"), counts):
            if count == 0:
                missing[(text, side)] += 1

    expected = Counter({key: count for key, (count, _) in KNOWN_UNREACHABLE.items()})
    uncovered = missing - expected
    obsolete = expected - missing
    covered = 2 * len(by_location) - sum(missing.values())
    print(f"Reducer branch outcomes: {covered}/{2 * len(by_location)} covered")
    print(f"Reviewed unreachable directions: {sum(missing.values())}")
    for (source, side), count in sorted(missing.items()):
        reason = KNOWN_UNREACHABLE.get((source, side), (0, "UNREVIEWED"))[1]
        print(f"  {count} x {side}: {source} - {reason}")
    for label, difference in (("Unreviewed missed", uncovered), ("Obsolete exception", obsolete)):
        for (source, side), count in sorted(difference.items()):
            print(f"{label}: {count} x {side}: {source}", file=sys.stderr)
    return 1 if uncovered or obsolete else 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: audit_reducer_branches.py <llvm-cov-export.json>")
    raise SystemExit(audit(Path(sys.argv[1])))
