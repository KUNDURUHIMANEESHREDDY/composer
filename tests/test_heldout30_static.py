"""Reproducible 30-held-out-repository STATIC analysis measurement.

Never runs, builds or imports upstream code. This is NOT a functional
compatibility benchmark: behavioral admission remains a separate release gate.
All pinned cases, including failures, count toward the denominator.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
COHORT = ROOT / "tests/heldout30_pins.json"
KNOWN_PRIOR = {
    "TheAlgorithms/Rust", "EbTech/rust-algorithms", "ijt/trigram",
    "weihanglo/rust-algorithm-club", "rapidfuzz/strsim-rs",
    "kornelski/deunicode", "mattgathu/slugify", "life4/textdistance.rs",
    "rust-lang/glob", "rust-lang/cfg-if", "rust-lang/rustc-demangle",
    "rust-lang/rustc-hash",
}
SHA = re.compile(r"^[a-f0-9]{40}$")
REPO = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")


def load_cohort():
    raw = json.loads(COHORT.read_text(encoding="utf-8"))
    repos = raw["repos"]
    assert raw["schema_version"] == 1 and len(repos) == 30
    assert "static" in raw["purpose"]
    assert len({entry["repo"].lower() for entry in repos}) == 30
    for entry in repos:
        assert set(entry) == {"repo", "sha"}, entry
        assert REPO.fullmatch(entry["repo"]) and SHA.fullmatch(entry["sha"]), entry
        assert entry["repo"].lower() not in {x.lower() for x in KNOWN_PRIOR}, entry
    return repos


def restricted_git_env():
    env = os.environ.copy()
    env.update(
        GIT_TERMINAL_PROMPT="0",
        GIT_CONFIG_NOSYSTEM="1",
        GIT_CONFIG_GLOBAL="NUL" if os.name == "nt" else "/dev/null",
        GIT_ATTR_NOSYSTEM="1",
        GIT_ALLOW_PROTOCOL="https",
        GIT_OPTIONAL_LOCKS="0",
    )
    for key in list(env):
        if key.startswith("GIT_CONFIG_") and key not in (
            "GIT_CONFIG_NOSYSTEM", "GIT_CONFIG_GLOBAL"
        ):
            env.pop(key, None)
    env.pop("GIT_EXTERNAL_DIFF", None)
    return env


def execute(args, timeout=120, *, cwd=None, git=False):
    output = subprocess.run(
        [str(arg) for arg in args], cwd=cwd, env=restricted_git_env() if git else None,
        capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout,
    )
    if output.returncode:
        # Bound returned diagnostics and avoid recording sensitive global paths.
        raise RuntimeError(
            f"{args[0]} exit {output.returncode}: {output.stderr[-350:]}"
        )
    return output.stdout.strip()


def analyze_one(entry, cli, scratch):
    started = time.monotonic()
    repo, rev = entry["repo"], entry["sha"]
    target = scratch / "source"
    report_a = scratch / "analysis-a.json"
    report_b = scratch / "analysis-b.json"
    evidence = {
        "repository": repo, "commit": rev,
        "static_analysis_passed": False,
        "functional_integration_verified": False,
        "method": "pinned_git_source_read_only_two_run_ast_analysis",
    }
    try:
        execute(["git", "init", "-q", "--template=", str(target)], git=True, timeout=20)
        execute(["git", "-C", target, "remote", "add", "origin",
                 f"https://github.com/{repo}.git"], git=True, timeout=20)
        execute(["git", "-C", target, "-c", "protocol.ext.allow=never",
                 "-c", "core.fsmonitor=false", "fetch", "--no-tags", "--depth=1",
                 "origin", rev], git=True, timeout=130)
        execute(["git", "-C", target, "-c", "core.fsmonitor=false",
                 "checkout", "--detach", "--force", "FETCH_HEAD"], git=True, timeout=45)
        observed = execute(["git", "-C", target, "rev-parse", "--verify",
                            "HEAD^{commit}"], git=True, timeout=20)
        assert observed.lower() == rev, "pinned checkout revision mismatch"
        tree = execute(["git", "-C", target, "rev-parse", "--verify",
                        "HEAD^{tree}"], git=True, timeout=20)
        assert SHA.fullmatch(tree), "missing immutable Git tree"
        execute([cli, "analyze", target, "--output", report_a], timeout=110)
        execute([cli, "analyze", target, "--output", report_b], timeout=110)
        first = report_a.read_bytes()
        assert first == report_b.read_bytes(), "analysis non-deterministic"
        graph = json.loads(first)
        summary = graph["summary"]
        count = summary["files_analyzed"]
        symbols = summary["symbols_found"]
        assert count > 0 and symbols > 0, "no parsed source symbols"
        files = graph["files"]
        assert len(files) == count, "graph file count inconsistent"
        tracked = set(execute(["git", "-C", target, "ls-files", "--cached"],
                              git=True, timeout=20).splitlines())
        # Symlinked/generated files must not escape the checked out Git source.
        for item in files:
            name = item["path"].replace("\\", "/")
            assert name in tracked, f"analyzed untracked file: {name}"
            assert (target / name).resolve().is_relative_to(target.resolve()), "path escape"
        status = execute(["git", "-C", target, "-c", "core.fsmonitor=false",
                          "status", "--porcelain", "--untracked-files=all"],
                         git=True, timeout=20)
        assert not status, "source modified during read-only analysis"
        evidence.update(
            static_analysis_passed=True, git_tree=tree,
            source_files_parsed=count, source_symbols_found=symbols,
            analyzed_graph_sha256=hashlib.sha256(first).hexdigest(),
            repeated_analysis_identical=True, original_checkout_clean=True,
        )
    except Exception as exc:
        evidence["failure"] = (type(exc).__name__ + ": " + str(exc))[-450:]
    evidence["elapsed_seconds"] = round(time.monotonic() - started, 3)
    return evidence


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--lint", action="store_true")
    parser.add_argument("--shard", type=int, choices=(0, 1, 2))
    parser.add_argument("--cli", default="target/release/composer")
    parser.add_argument("--report")
    args = parser.parse_args()
    cohort = load_cohort()
    if args.lint:
        print("PASS frozen, disjoint, immutable 30-repo cohort")
        return 0
    assert args.shard is not None and args.report, "shard and report required"
    chosen = cohort[args.shard * 10: (args.shard + 1) * 10]
    cli = Path(args.cli).resolve()
    assert cli.is_file(), f"Composer compiled CLI unavailable: {cli}"
    results = []
    for entry in chosen:
        with tempfile.TemporaryDirectory(prefix=f"composer-heldout30-{args.shard}-") as tmp:
            result = analyze_one(entry, cli, Path(tmp))
        results.append(result)
        print(json.dumps(result, sort_keys=True), flush=True)
    record = {
        "schema_version": 1,
        "cohort_sha256": hashlib.sha256(COHORT.read_bytes()).hexdigest(),
        "shard": args.shard, "cases": len(results),
        "static_analysis_successes": sum(x["static_analysis_passed"] for x in results),
        "functional_integrations_verified": 0,
        "limitations": "Static Git/Tree-sitter analysis ONLY; none of these are "
                       "independently behaviorally verified integrations. "
                       "General compatibility and production readiness remain unproven.",
        "results": results,
    }
    report = Path(args.report)
    report.parent.mkdir(parents=True, exist_ok=True)
    report.write_text(json.dumps(record, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    print("EVIDENCE_REPORT " + str(report), flush=True)
    return 0 if record["static_analysis_successes"] == len(chosen) else 1


if __name__ == "__main__":
    sys.exit(main())
