"""Holdout external Git-only acceptance: different upstream, API and target.

Uses independent third-party code with Cargo native Git dependencies; does not
regenerate algorithms or execute unreviewed code in the host process.
"""
from __future__ import annotations

import json
import shutil
import tempfile
from pathlib import Path

from test_unpublished_git_public import (
    GIT_CACHE, CACHE, cli, verify_unpublished_on_registry as verify_original_absence,
    main as run_first_unpublished_upstream,
)
from test_pinned_public_multifile import clone_exact, run

UPSTREAM = "https://github.com/weihanglo/rust-algorithm-club.git"
UPSTREAM_SHA = "1954301f21d44c18ea44864a776a3f6fd72eaefe"
PACKAGE = "rust-algorithm-club"
TARGET = "https://github.com/rapidfuzz/strsim-rs.git"
TARGET_SHA = "dacc84c0dc61eff0ee0ff66962bcf2e17018ad26"
CONTRACT = "tests/composer_levenshtein_external_contract.rs"
TESTS = """use strsim::ported_levenshtein;
#[test]
fn independent_edit_distance_oracle() {
    assert_eq!(ported_levenshtein("", ""), 0);
    assert_eq!(ported_levenshtein("kitten", "sitting"), 3);
    assert_eq!(ported_levenshtein("saturday", "sunday"), 3);
    assert_eq!(ported_levenshtein("rust", "rust"), 0);
    assert_eq!(ported_levenshtein("abc", "xyz"), 3);
    assert_eq!(ported_levenshtein("cat", "cut"), 1);
    assert_eq!(ported_levenshtein("café", "cafe"), 1);
}
"""


def verify_absent(package: str) -> None:
    # Distinguish an actual 404 from a network/transient failure.
    import urllib.error
    import urllib.request
    url = "https://crates.io/api/v1/crates/" + package
    request = urllib.request.Request(url, headers={
        "User-Agent": "Composer-public-Git-multisource-acceptance/1.0",
        "Accept": "application/json",
    })
    try:
        with urllib.request.urlopen(request, timeout=18) as response:
            raise AssertionError(
                f"candidate {package} published on crates.io (HTTP {response.status})"
            )
    except urllib.error.HTTPError as error:
        assert error.code == 404, f"registry response not proof of unpublished: {error.code}"


def prepare(target: Path, root: Path) -> None:
    pref = root / "cargo-git-prefetch"
    shutil.copytree(target, pref, ignore=shutil.ignore_patterns(".git"))
    manifest = pref / "Cargo.toml"
    old = manifest.read_text()
    assert "[dependencies]" in old
    manifest.write_text(old.replace(
        "[dependencies]",
        '[dependencies]\ncomposer_git_rust_algorithm_club = { package = "rust-algorithm-club", '
        f'git = "{UPSTREAM}", rev = "{UPSTREAM_SHA}" }}',
        1,
    ))
    run("cargo", "fetch", "--manifest-path", str(manifest),
        "--target", "x86_64-unknown-linux-gnu", timeout=180)
    assert (GIT_CACHE / "checkouts").is_dir() and CACHE.is_dir()


def target_copy(original: Path, base: Path, label: str):
    target = base / ("target-" + label)
    shutil.copytree(original, target, ignore=shutil.ignore_patterns(".git"))
    (target / "tests").mkdir(exist_ok=True)
    (target / CONTRACT).write_text(TESTS, encoding="utf-8")
    snapshots = {p: (target / p).read_bytes() for p in [
        "Cargo.toml", "src/lib.rs", CONTRACT
    ]}
    return target, snapshots


def unchanged(target: Path, snapshots: dict[str, bytes]) -> None:
    for path, before in snapshots.items():
        assert (target / path).read_bytes() == before, f"original changed: {path}"


def verify(target: Path, base: Path, checkout: Path, label: str,
           symbol: str, require_success: bool = True):
    out = base / ("output-" + label)
    response = cli(
        "verify-external-git-rust", str(target),
        "--output", str(out),
        "--upstream-checkout", str(checkout),
        "--upstream-repository", UPSTREAM[:-4],
        "--upstream-commit", UPSTREAM_SHA,
        "--package", PACKAGE,
        "--public-symbol", symbol,
        "--target-alias", "ported_levenshtein",
        "--contract-test", CONTRACT,
        "--registry-cache", str(CACHE),
        "--git-cache", str(GIT_CACHE),
        "--license-reviewed",
        require_success=require_success,
    )
    return response, out


def test_independent_second_source() -> dict:
    verify_absent(PACKAGE)
    with tempfile.TemporaryDirectory(prefix="composer-holdout-git-only-") as tmp:
        base = Path(tmp)
        checkout = base / "weihanglo-rust-algorithm-club"
        source_target = base / "rapidfuzz-strsim"
        clone_exact(UPSTREAM, UPSTREAM_SHA, checkout)
        clone_exact(TARGET, TARGET_SHA, source_target)
        assert (checkout / "LICENSE").is_file()
        assert (source_target / "src/lib.rs").is_file()
        assert (checkout / "src/levenshtein_distance/mod.rs").is_file()
        prepare(source_target, base)
        evidence = []
        for symbol in ["levenshtein_distance", "levenshtein_distance_naive"]:
            target, snapshots = target_copy(source_target, base, symbol)
            result, out = verify(target, base, checkout, symbol, symbol)
            report = json.loads(result.stdout)
            assert report["offline_tests_passed"] is True
            assert report["source_git_objects_verified"] is True
            assert report["reviewed_source_tree_oid"]
            assert report["reviewed_manifest_blob_oid"]
            assert report["reviewed_license_blob_oid"]
            assert report["source_checkout_commit_verified"]
            assert report["lockfile_git_commit_verified"]
            assert (out / "Cargo.lock").is_file()
            assert UPSTREAM_SHA in (out / "Cargo.lock").read_text()
            assert f"pub use composer_git_rust_algorithm_club::{symbol} as ported_levenshtein" in (
                out / "src/lib.rs"
            ).read_text()
            proof = json.loads((out / "composer-upstream-git-evidence.json").read_text())
            assert proof["source_git_objects_verified"] is True
            assert proof["reviewed_source_tree_oid"] == report["reviewed_source_tree_oid"]
            assert proof["reviewed_manifest_blob_oid"] == report["reviewed_manifest_blob_oid"]
            assert proof["reviewed_license_blob_oid"] == report["reviewed_license_blob_oid"]
            assert proof["network_disabled"] and proof["offline_tests_passed"]
            assert "test result: ok" in proof["log_tail"]
            assert (out / "third_party_licenses/rust-algorithm-club-git-license.txt").read_bytes() == (
                checkout / "LICENSE"
            ).read_bytes()
            unchanged(target, snapshots)
            evidence.append({"symbol": symbol, "verified": True,
                             "elapsed_seconds": report["elapsed_seconds"]})
            print(f"PASS second unrelated Git-only repo -> independent target: {symbol}",
                  flush=True)

        target, snapshots = target_copy(source_target, base, "bad-hamming")
        fail, output = verify(
            target, base, checkout, "bad-hamming", "hamming_distance_str",
            require_success=False,
        )
        assert fail.returncode != 0
        assert "test result: FAILED" in fail.stderr, fail.stderr[-1800:]
        assert not output.exists()
        unchanged(target, snapshots)
        print("PASS wrong but type-compatible different Git source behavior rejected", flush=True)

        return {"repo": UPSTREAM, "sha": UPSTREAM_SHA, "published_on_crates_io": False,
                "target": TARGET, "target_sha": TARGET_SHA,
                "source_license_evidence": "preserved",
                "validated_git_objects": True,
                "successful_imports": evidence, "wrong_api_refused": True,
                "originals_unchanged": True}


def main() -> None:
    run("docker", "image", "inspect", "rust:1.99-slim")
    # Retain the first independent source as a control, so this acceptance
    # tests multiple DIFFERENT repositories and unrelated targets in one run.
    run_first_unpublished_upstream()
    new = test_independent_second_source()
    print("EVIDENCE_JSON " + json.dumps({
        "different_git_only_upstreams": 2,
        "different_target_repositories": 2,
        "verified_git_only_imports": 3,
        "independent_second_source": new,
        "original_tests_and_provenance_gate_unchanged": True,
        "limitations": "Two pinned public Git-only projects and two unrelated targets "
                       "are not an unbiased generalization statistic. Build attestation "
                       "covers Composer's Windows executable, not upstream source reputation.",
    }, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
