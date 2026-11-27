"""Independent held-out target for Composer's existing, pinned Git-only importer.

The first target in PR #26 was ijt/trigram. This acceptance transfers the
same upstream totient capability to a *different* unrelated existing library,
weihanglo/rust-algorithm-club, without reimplementing upstream source code.
"""
from __future__ import annotations

import json
import shutil
import tempfile
from pathlib import Path

from test_pinned_public_multifile import CACHE, clone_exact, run
from test_unpublished_git_public import (
    UPSTREAM_URL, UPSTREAM_SHA, PACKAGE, GIT_CACHE, cli
)

SECOND_TARGET = "https://github.com/weihanglo/rust-algorithm-club.git"
SECOND_TARGET_SHA = "1954301f21d44c18ea44864a776a3f6fd72eaefe"
CONTRACT = "tests/composer_second_target_totient.rs"
CONTRACT_SOURCE = """use rust_algorithm_club::ported_totient;
#[test]
fn independent_totient_oracle() {
    for (n, expected) in [(1, 1), (2, 1), (9, 6),
                          (10, 4), (13, 12), (36, 12), (100, 40)] {
        assert_eq!(ported_totient(n), expected, "totient({n})");
    }
}
"""


def prefetch_cargo_source(target: Path, root: Path):
    prefetch = root / "prefetch"
    shutil.copytree(target, prefetch, ignore=shutil.ignore_patterns(".git"))
    manifest = prefetch / "Cargo.toml"
    data = manifest.read_text()
    assert "[package]" in data
    data += (
        '\n[dependencies]\n'
        f'composer_git_the_algorithms_rust = {{ package = "{PACKAGE}", '
        f'git = "{UPSTREAM_URL}", rev = "{UPSTREAM_SHA}" }}\n'
    )
    manifest.write_text(data)
    # This historical target pins libc=0.2.76, incompatible with the
    # upstream's newer rand/getrandom requirements. Cargo resolves a fresh
    # compatible graph ONLY in this disposable prefetch copy, never in the
    # reviewed original checkout. The resulting exact lock is preserved in
    # the independent Docker verifier.
    stale_lock = prefetch / "Cargo.lock"
    if stale_lock.exists():
        stale_lock.unlink()
    run("cargo", "fetch", "--manifest-path", str(manifest),
        "--target", "x86_64-unknown-linux-gnu", timeout=180)
    assert (prefetch / "Cargo.lock").is_file(), "Cargo did not produce prefetch lock"
    assert GIT_CACHE.is_dir() and CACHE.is_dir()
    return prefetch / "Cargo.lock"


def make_disposable_target(original: Path, root: Path, label: str,
                           prefetched_lockfile: Path):
    target = root / f"target-{label}"
    shutil.copytree(original, target, ignore=shutil.ignore_patterns(".git"))
    (target / "tests").mkdir(exist_ok=True)
    (target / CONTRACT).write_text(CONTRACT_SOURCE, encoding="utf-8")
    shutil.copy2(prefetched_lockfile, target / "Cargo.lock")
    snapshot = {
        name: (target / name).read_bytes()
        for name in ["Cargo.toml", "Cargo.lock", "src/lib.rs", CONTRACT]
    }
    return target, snapshot


def call_verifier(target: Path, source: Path, root: Path, label: str,
                  public_symbol: str = "number_theory::euler_totient",
                  require_success=True):
    output = root / f"accepted-{label}"
    result = cli(
        "verify-external-git-rust", str(target),
        "--output", str(output),
        "--upstream-checkout", str(source),
        "--upstream-repository", UPSTREAM_URL[:-4],
        "--upstream-commit", UPSTREAM_SHA,
        "--package", PACKAGE,
        "--public-symbol", public_symbol,
        "--target-alias", "ported_totient",
        "--contract-test", CONTRACT,
        "--registry-cache", str(CACHE),
        "--git-cache", str(GIT_CACHE),
        "--license-reviewed",
        require_success=require_success,
    )
    return result, output


def same_bytes(original: Path, snapshot):
    for name, before in snapshot.items():
        assert (original / name).read_bytes() == before, name


def main():
    run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-second-unrelated-target-") as dirname:
        base = Path(dirname)
        upstream = base / "thealgorithms-pinned"
        original_target = base / "weihanglo-pinned"
        clone_exact(UPSTREAM_URL, UPSTREAM_SHA, upstream)
        clone_exact(SECOND_TARGET, SECOND_TARGET_SHA, original_target)
        assert (original_target / "LICENSE").is_file()
        assert (upstream / "LICENSE").is_file()
        assert (original_target / "src/lib.rs").is_file()
        lockfile = prefetch_cargo_source(original_target, base)

        target, baseline = make_disposable_target(original_target, base, "positive", lockfile)
        result, output = call_verifier(target, upstream, base, "positive")
        evidence = json.loads(result.stdout)
        assert evidence["offline_tests_passed"] is True
        assert evidence["source_git_objects_verified"] is True
        assert evidence["lockfile_git_commit_verified"] is True
        assert evidence["source_checkout_commit_verified"] is True
        assert evidence["upstream_commit"] == UPSTREAM_SHA
        assert len(evidence["source_license_sha256"]) == 64
        assert len(evidence["reviewed_manifest_blob_oid"]) == 40
        assert len(evidence["reviewed_license_blob_oid"]) == 40
        assert (output / "third_party_licenses/the_algorithms_rust-git-license.txt").read_bytes() == (
            upstream / "LICENSE"
        ).read_bytes()
        assert "pub use composer_git_the_algorithms_rust::number_theory::euler_totient as ported_totient" in (
            output / "src/lib.rs"
        ).read_text()
        assert UPSTREAM_SHA in (output / "Cargo.lock").read_text()
        proof = json.loads((output / "composer-upstream-git-evidence.json").read_text())
        assert proof["source_git_objects_verified"] is True
        assert proof["network_disabled"] is True
        assert proof["offline_tests_passed"] is True
        assert "test result: ok" in proof["log_tail"]
        same_bytes(target, baseline)
        print("PASS second unrelated target with exact upstream Git algorithm", flush=True)

        negative, original_bytes = make_disposable_target(
            original_target, base, "negative", lockfile
        )
        response, rejected = call_verifier(
            negative, upstream, base, "negative",
            public_symbol="math::factorial::factorial",
            require_success=False,
        )
        assert response.returncode != 0
        assert "test result: FAILED" in response.stderr, response.stderr[-2500:]
        assert not rejected.exists()
        same_bytes(negative, original_bytes)
        print("PASS incorrect but type-compatible upstream rejected with no mutation",
              flush=True)

        print("EVIDENCE_JSON " + json.dumps({
            "source": UPSTREAM_URL, "source_commit": UPSTREAM_SHA,
            "original_target": SECOND_TARGET, "target_commit": SECOND_TARGET_SHA,
            "independent_target": True, "verified_successes": 1,
            "semantic_control_refused": True, "source_git_objects_verified": True,
            "license_preserved": True, "offline_docker": True,
            "originals_unchanged": True,
            "limitations": "Two curated public Rust targets are not general "
                           "reliability across unfamiliar repositories."
        }, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
