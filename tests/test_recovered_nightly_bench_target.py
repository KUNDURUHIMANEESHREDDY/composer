"""Regression of a REAL prior failure: a third-party library uses nightly-only benches.

Pinned rapidfuzz/strsim-rs's benches/benches.rs contains #![feature(test)],
which cannot compile on the stable Rust Docker verifier. Composer must NOT
silently downgrade its default all-targets assurance. Explicitly opt into
library+integration tests, independently test the *existing* Git upstream,
repeat three independent fresh target copies, and reject wrong semantics.
"""
from __future__ import annotations

import json
import shutil
import tempfile
from pathlib import Path

from test_pinned_public_multifile import CACHE, clone_exact, run
from test_unpublished_git_public import UPSTREAM_URL, UPSTREAM_SHA, PACKAGE, GIT_CACHE, cli

TARGET_URL = "https://github.com/rapidfuzz/strsim-rs.git"
TARGET_SHA = "dacc84c0dc61eff0ee0ff66962bcf2e17018ad26"
CONTRACT = "tests/composer_holdout_totient_contract.rs"
TESTS = """use strsim::ported_totient;
#[test]
fn independent_euler_totient_behavior() {
    for (input, expected) in [(1, 1), (2, 1), (9, 6),
                              (10, 4), (13, 12), (36, 12), (100, 40)] {
        assert_eq!(ported_totient(input), expected, "phi({input})");
    }
}
"""


def prefetch(target: Path, base: Path) -> Path:
    copied = base / "cargo-only-prefetch"
    shutil.copytree(target, copied, ignore=shutil.ignore_patterns(".git"))
    manifest = copied / "Cargo.toml"
    source = manifest.read_text()
    assert "[package]" in source
    addition = (
        '\n[dependencies]\n'
        f'composer_git_the_algorithms_rust = {{ package = "{PACKAGE}", '
        f'git = "{UPSTREAM_URL}", rev = "{UPSTREAM_SHA}" }}\n'
    )
    assert "[dependencies]" not in source
    manifest.write_text(source.rstrip() + "\n" + addition)
    stale = copied / "Cargo.lock"
    if stale.exists():
        stale.unlink()
    # Cargo fetch is metadata/archive acquisition only; not host execution.
    run("cargo", "fetch", "--manifest-path", str(manifest),
        "--target", "x86_64-unknown-linux-gnu", timeout=180)
    assert (copied / "Cargo.lock").is_file()
    assert GIT_CACHE.is_dir() and CACHE.is_dir()
    return copied / "Cargo.lock"


def target_copy(original: Path, base: Path, key: str, pinned_lock: Path):
    target = base / f"holdout-{key}"
    shutil.copytree(original, target, ignore=shutil.ignore_patterns(".git"))
    (target / "tests").mkdir(exist_ok=True)
    (target / CONTRACT).write_text(TESTS)
    shutil.copy2(pinned_lock, target / "Cargo.lock")
    baseline = {
        p: (target / p).read_bytes()
        for p in ["Cargo.toml", "Cargo.lock", "src/lib.rs", CONTRACT]
    }
    return target, baseline


def verify(target: Path, upstream: Path, base: Path, label: str,
           focused: bool, symbol="number_theory::euler_totient",
           require_success=True):
    output = base / ("verified-" + label)
    args = [
        "verify-external-git-rust", str(target),
        "--output", str(output),
        "--upstream-checkout", str(upstream),
        "--upstream-repository", UPSTREAM_URL[:-4],
        "--upstream-commit", UPSTREAM_SHA,
        "--package", PACKAGE,
        "--public-symbol", symbol,
        "--target-alias", "ported_totient",
        "--contract-test", CONTRACT,
        "--registry-cache", str(CACHE),
        "--git-cache", str(GIT_CACHE),
        "--license-reviewed",
    ]
    if focused:
        args.append("--contract-focused")
    return cli(*args, require_success=require_success), output


def assert_unchanged(original: Path, baseline: dict):
    for path, content in baseline.items():
        assert (original / path).read_bytes() == content, path


def main():
    run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-nightly-bench-repro-") as temp:
        base = Path(temp)
        upstream = base / "the-algorithms-reviewed"
        target_source = base / "real-rapidfuzz-strsim"
        clone_exact(UPSTREAM_URL, UPSTREAM_SHA, upstream)
        clone_exact(TARGET_URL, TARGET_SHA, target_source)

        bench = (target_source / "benches/benches.rs").read_text()
        assert "#![feature(test)]" in bench, "Pinned baseline must reproduce nightly-only bench"
        lock = prefetch(target_source, base)

        # First reproduce the previously observed failure without weakening
        # the default. Its cause must be the actual third-party benchmark.
        target, baseline = target_copy(target_source, base, "baseline", lock)
        failure, output = verify(target, upstream, base, "baseline", focused=False,
                                 require_success=False)
        assert failure.returncode != 0, "Default all-targets unexpectedly stopped rejecting nightly bench"
        assert ("feature" in failure.stderr.lower() or "E0554" in failure.stderr
                or "stable" in failure.stderr.lower()), failure.stderr[-3500:]
        assert not output.exists()
        assert_unchanged(target, baseline)
        print("PASS reproduced previous nightly-bench all-targets failure; original unchanged",
              flush=True)

        successful = []
        for attempt in range(1, 4):
            target, baseline = target_copy(target_source, base, f"repeat-{attempt}", lock)
            result, output = verify(target, upstream, base, f"repeat-{attempt}", focused=True)
            record = json.loads(result.stdout)
            assert record["offline_tests_passed"] is True
            assert record["verification_scope"] == "library_and_integration_tests"
            assert record["source_git_objects_verified"] is True
            assert record["lockfile_git_commit_verified"] is True
            evidence = json.loads(
                (output / "composer-upstream-git-evidence.json").read_text()
            )
            assert evidence["verification_scope"] == "library_and_integration_tests"
            assert evidence["third_party_benchmarks_verified"] is False
            assert evidence["network_disabled"] is True
            assert evidence["offline_tests_passed"] is True
            assert "test result: ok" in evidence["log_tail"]
            assert (output / "third_party_licenses/the_algorithms_rust-git-license.txt").read_bytes() == (
                upstream / "LICENSE"
            ).read_bytes()
            assert UPSTREAM_SHA in (output / "Cargo.lock").read_text()
            assert_unchanged(target, baseline)
            successful.append(record["elapsed_seconds"])
            print(f"PASS scoped integration fresh-run {attempt}/3; "
                  f"duration={record['elapsed_seconds']:.2f}s", flush=True)

        target, baseline = target_copy(target_source, base, "wrong-semantics", lock)
        negative, output = verify(
            target, upstream, base, "wrong-semantics", focused=True,
            symbol="math::factorial::factorial", require_success=False,
        )
        assert negative.returncode != 0
        assert "test result: FAILED" in negative.stderr, negative.stderr[-3000:]
        assert not output.exists()
        assert_unchanged(target, baseline)
        print("PASS focused verification rejected wrong but compilable upstream behavior",
              flush=True)

        print("EVIDENCE_JSON " + json.dumps({
            "upstream": UPSTREAM_URL, "upstream_commit": UPSTREAM_SHA,
            "previously_failing_target": TARGET_URL,
            "target_commit": TARGET_SHA,
            "default_all_targets": "failed_on_nightly_third_party_bench",
            "scoped_repeated_successes": len(successful),
            "scoped_repeated_attempts": 3,
            "wrong_behavior_rejected": True,
            "published_output_only_on_verified_tests": True,
            "originals_preserved": True,
            "network_disabled": True,
            "verification_scope": "library_and_integration_tests",
            "benchmarks_verified": False,
            "limits": "One new independent target with a nightly-only benchmark. "
                      "Focused mode does not establish success of all executable, "
                      "example and benchmark targets or arbitrary repository compatibility.",
        }, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
