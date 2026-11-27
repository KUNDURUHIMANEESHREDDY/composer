"""Fail-closed audit of the frozen 30-repository BEHAVIORAL test assignments.

This script checks only that each pinned target has a real functional job and
that the shared independent oracle meets the admission fixture shape.
It never reports behavioral success: that requires all 30 Docker-based jobs.
"""
from __future__ import annotations

import ast
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
TESTS = ROOT / "tests"
WORKFLOWS = ROOT / ".github" / "workflows"

# One base case + the hyperfine binary case, with 28 cases from the existing
# script case tables. Keep all integration behavior inside those test scripts.
CASES = {
    "test_heldout30_functional_anyhow.py": {"anyhow": "dtolnay/anyhow"},
    "test_heldout30_functional_hyperfine.py": {"hyperfine": "sharkdp/hyperfine"},
}


def constants_from_script(filename: str) -> dict[str, object]:
    tree = ast.parse((TESTS / filename).read_text(encoding="utf-8"), filename=filename)
    result: dict[str, object] = {}
    for node in tree.body:
        if isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name) and target.id in {"CASES", "INDEPENDENT_ORACLE"}:
                    result[target.id] = ast.literal_eval(node.value)
    return result


def audit() -> None:
    all_cases: dict[str, tuple[str, str]] = {}
    for filename, base in CASES.items():
        for name, repo in base.items():
            assert repo not in all_cases, f"duplicate repository: {repo}"
            all_cases[repo] = (filename, name)

    functional_tables = [
        "test_heldout30_functional_standalone.py",
        "test_heldout30_functional_virtual_members.py",
        "test_heldout30_functional_workspace_root.py",
        "test_heldout30_functional_extra.py",
    ]
    for filename in functional_tables:
        cases = constants_from_script(filename)["CASES"]
        assert isinstance(cases, dict) and cases, f"no cases in {filename}"
        for name, value in cases.items():
            repo = value[0]
            assert repo not in all_cases, f"duplicate repository: {repo}"
            all_cases[repo] = (filename, name)

    cohort = json.loads((TESTS / "heldout30_pins.json").read_text(encoding="utf-8"))
    pinned = {item["repo"] for item in cohort["repos"]}
    assert len(cohort["repos"]) == len(pinned) == 30, "cohort must remain 30 unique pinned repos"
    assert set(all_cases) == pinned, (
        "Functional case coverage differs from frozen 30: "
        f"missing={sorted(pinned - set(all_cases))} extra={sorted(set(all_cases) - pinned)}"
    )

    workflow_by_table = {
        "test_heldout30_functional_anyhow.py": "heldout30-functional-anyhow.yml",
        "test_heldout30_functional_hyperfine.py": "heldout30-functional-hyperfine.yml",
        "test_heldout30_functional_standalone.py": "heldout30-functional-standalone.yml",
        "test_heldout30_functional_virtual_members.py": "heldout30-functional-virtual-members.yml",
        "test_heldout30_functional_workspace_root.py": "heldout30-functional-workspace-root.yml",
        "test_heldout30_functional_extra.py": "heldout30-functional-extra.yml",
    }
    for filename, workflow in workflow_by_table.items():
        content = (WORKFLOWS / workflow).read_text(encoding="utf-8")
        assert filename in content, f"{workflow} doesn't invoke its behavioral harness"
        expected = {name for repo, (source, name) in all_cases.items() if source == filename}
        if filename in CASES:
            # The single-target case uses its existing script's fixed target.
            assert "python3 -u tests/" + filename in content, workflow
        else:
            observed = set(re.findall(r"(?m)^\s*- case:\s*([\w-]+)\s*$", content))
            assert observed == expected, (
                f"{workflow} job matrix mismatch: expected {sorted(expected)}, got {sorted(observed)}"
            )
            assert "--target-case" in content, f"case arguments not passed in {workflow}"
    oracle = constants_from_script("test_heldout30_functional_anyhow.py")["INDEPENDENT_ORACLE"]
    assert isinstance(oracle, str)
    unconditional = oracle.split("    for (n, expected) in ", 1)[0]
    assert re.search(r"(?m)^\s{4}assert_eq!\(ported_totient\(36\), 12\);$", unconditional), (
        "Shared functional oracle lost its unconditional direct API assertion"
    )
    for expected in ("(1, 1)", "(2, 1)", "(3, 2)", "(9, 6)", "(10, 4)", "(13, 12)", "(36, 12)", "(100, 40)"):
        assert expected in oracle, f"independent behavioral case missing: {expected}"
    print(
        "PASS 30 distinct pinned repository targets are assigned to real behavioral CI jobs; "
        "unconditional assertion and all eight oracle cases retained. "
        "This is coverage only, NOT 30/30 functional verification."
    )


if __name__ == "__main__":
    audit()
