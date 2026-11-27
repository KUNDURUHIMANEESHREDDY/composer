"""Adopt third-party published Rust functionality without reimplementing source.

Source = independent rapidfuzz/strsim-rs (crates.io strsim 0.11.1).
Target = unrelated public ijt/trigram. Both are pinned to exact Git SHAs.
Cargo resolves the upstream published crate; execution is restricted to Composer's
existing offline Docker verifier, with manually authored target-owned tests.
"""
from __future__ import annotations

import json
from pathlib import Path
import shutil
import tempfile

from test_pinned_public_multifile import CACHE, clone_exact, run

SOURCE_REPO = "https://github.com/rapidfuzz/strsim-rs.git"
SOURCE_SHA = "dacc84c0dc61eff0ee0ff66962bcf2e17018ad26"
TARGET_REPO = "https://github.com/ijt/trigram.git"
TARGET_SHA = "45af254dcd7b5c8aac4feb336192204791cd1e2f"
PINNED_VERSION = "0.11.1"
TEST_NAME = "tests/composer_external_contract.rs"
REPEATS = 3


def cli(*args: str, require_success: bool = True):
    return run("cargo", "run", "--quiet", "-p", "composer-cli", "--",
               *args, require_success=require_success, timeout=300)


def test_fixture(target: Path, root: Path, name: str, contract: str):
    work = root / f"target-{name}"
    shutil.copytree(target, work, ignore=shutil.ignore_patterns(".git"))
    (work / "tests").mkdir(exist_ok=True)
    (work / TEST_NAME).write_text(
        "use trigram::ported_jaro;\n" + contract,
        encoding="utf-8",
    )
    manifest = (work / "Cargo.toml").read_bytes()
    library = (work / "src/lib.rs").read_bytes()
    test = (work / TEST_NAME).read_bytes()
    return work, (manifest, library, test)


def invoke(target: Path, dest: Path, require_success: bool = True, **overrides):
    params = {
        "package": "strsim",
        "version": PINNED_VERSION,
        "public_symbol": "jaro",
        "target_alias": "ported_jaro",
        "upstream_repository": SOURCE_REPO[:-4],
        "upstream_commit": SOURCE_SHA,
    }
    params.update(overrides)
    return cli(
        "verify-external-rust-crate", str(target),
        "--output", str(dest),
        "--package", params["package"],
        "--version", params["version"],
        "--public-symbol", params["public_symbol"],
        "--target-alias", params["target_alias"],
        "--contract-test", TEST_NAME,
        "--registry-cache", str(CACHE),
        "--upstream-repository", params["upstream_repository"],
        "--upstream-commit", params["upstream_commit"],
        "--license-reviewed",
        require_success=require_success,
    )


GOOD_CONTRACT = """#[test]
fn third_party_jaro_is_the_requested_algorithm() {
    assert!((ported_jaro("", "") - 1.0).abs() < 1e-12);
    assert!((ported_jaro("same", "same") - 1.0).abs() < 1e-12);
    assert!(ported_jaro("abc", "xyz").abs() < 1e-12);
    assert!((ported_jaro("MARTHA", "MARHTA") - 17.0 / 18.0).abs() < 1e-12);
    assert!((ported_jaro("🐱", "🐱") - 1.0).abs() < 1e-12);
}
"""

MISLEADING_CONTRACT = """#[test]
fn invalid_jaro_winkler_expectation() {
    assert!(ported_jaro("MARTHA", "MARHTA") > 0.95);
}
"""


def original_unchanged(target: Path, snapshots: tuple[bytes, bytes, bytes]) -> None:
    assert (target / "Cargo.toml").read_bytes() == snapshots[0]
    assert (target / "src/lib.rs").read_bytes() == snapshots[1]
    assert (target / TEST_NAME).read_bytes() == snapshots[2]


def prepare_registry_cache(target: Path, root: Path) -> None:
    """Only retrieves registry artifacts. This does not execute upstream build scripts."""
    prefetch = root / "prefetch"
    shutil.copytree(target, prefetch, ignore=shutil.ignore_patterns(".git"))
    manifest = prefetch / "Cargo.toml"
    original = manifest.read_text()
    assert "[dependencies]" in original
    manifest.write_text(original.replace(
        "[dependencies]", '[dependencies]\nstrsim = "=0.11.1"', 1
    ))
    run("cargo", "fetch", "--manifest-path", str(manifest),
        "--target", "x86_64-unknown-linux-gnu", timeout=180)
    assert CACHE.exists(), "Cargo registry cache missing"


def test_successes(target: Path, root: Path) -> list[dict]:
    results: list[dict] = []
    target_baseline = (target / "src/lib.rs").read_bytes()
    for attempt in range(1, REPEATS + 1):
        work, snapshots = test_fixture(target, root, f"accepted-{attempt}", GOOD_CONTRACT)
        out = root / f"accepted-output-{attempt}"
        result = json.loads(invoke(work, out).stdout)
        assert result["offline_tests_passed"] is True
        assert result["original_target_unchanged"] is True
        assert result["locked_version"] == PINNED_VERSION
        assert result["target_alias"] == "ported_jaro"
        assert result["provenance_commit_matched_to_registry_artifact"] is False
        assert len(result["registry_checksum"]) == 64
        evidence = json.loads((out / "composer-upstream-crate-evidence.json").read_text())
        assert evidence["network_disabled"] is True
        assert evidence["offline_tests_passed"] is True
        assert evidence["pinned_version"] == PINNED_VERSION
        assert "test result: ok" in evidence["log_tail"]
        assert 'pub use strsim::jaro as ported_jaro;' in (out / "src/lib.rs").read_text()
        assert '=0.11.1' in (out / "Cargo.toml").read_text()
        original_unchanged(work, snapshots)
        assert (target / "src/lib.rs").read_bytes() == target_baseline
        print(f"PASS upstream-reuse iteration={attempt}/{REPEATS} "
              f"docker_s={result['elapsed_seconds']:.2f}", flush=True)
        results.append({"iteration": attempt, "verified": True,
                        "docker_seconds": result["elapsed_seconds"]})
    return results


def test_rejections(target: Path, root: Path) -> None:
    wrong, snapshots = test_fixture(target, root, "wrong-behavior", MISLEADING_CONTRACT)
    out = root / "must-not-publish-wrong-behavior"
    result = invoke(wrong, out, require_success=False)
    assert result.returncode != 0, "type-compatible but wrong behavior was accepted"
    assert "test result: FAILED" in result.stderr, result.stderr[-2500:]
    assert not out.exists(), "wrong behavior published an output"
    original_unchanged(wrong, snapshots)
    print("PASS wrong-but-compilable upstream behavior rejected by contract", flush=True)

    conflict, snapshots = test_fixture(target, root, "dependency-conflict", GOOD_CONTRACT)
    cargo = conflict / "Cargo.toml"
    cargo.write_text(cargo.read_text().replace(
        "[dependencies]", '[dependencies]\nstrsim = "=0.10.0"', 1
    ))
    snapshot_changed = (cargo.read_bytes(), snapshots[1], snapshots[2])
    out = root / "must-not-publish-conflict"
    result = invoke(conflict, out, require_success=False)
    assert result.returncode != 0, "existing incompatible dependency overwritten"
    assert "already declares strsim" in result.stderr, result.stderr[-2500:]
    assert not out.exists()
    original_unchanged(conflict, snapshot_changed)
    print("PASS conflicting existing crate requirement refused with no mutations", flush=True)

    path, snapshots = test_fixture(target, root, "invalid-version", GOOD_CONTRACT)
    out = root / "must-not-publish-invalid"
    result = invoke(path, out, require_success=False, version="^0.11")
    assert result.returncode != 0
    assert "exact crate version required" in result.stderr, result.stderr[-2000:]
    assert not out.exists()
    original_unchanged(path, snapshots)
    print("PASS non-pinned version rejected before execution", flush=True)


def main() -> None:
    run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-external-upstream-") as temp:
        root = Path(temp)
        source, target = root / "real-upstream", root / "real-target"
        clone_exact(SOURCE_REPO, SOURCE_SHA, source)
        clone_exact(TARGET_REPO, TARGET_SHA, target)
        import tomllib
        manifest = tomllib.loads((source / "Cargo.toml").read_text())
        assert manifest["package"]["name"] == "strsim"
        assert manifest["package"]["version"] == PINNED_VERSION
        assert "MIT" in manifest["package"]["license"]
        assert (source / "LICENSE").is_file()
        assert (target / "LICENSE").is_file()
        prepare_registry_cache(target, root)
        successes = test_successes(target, root)
        test_rejections(target, root)
        print("EVIDENCE_JSON " + json.dumps({
            "source_repo": SOURCE_REPO, "source_sha": SOURCE_SHA,
            "target_repo": TARGET_REPO, "target_sha": TARGET_SHA,
            "reused_crate": "strsim", "reused_crate_version": PINNED_VERSION,
            "attempts": REPEATS, "successes": len(successes),
            "runs": successes,
            "wrong_semantics_refused": True,
            "preexisting_dependency_conflict_refused": True,
            "unversioned_dependency_refused": True,
            "limitations": "Uses a published crates.io artifact of a public independent GitHub project; "
                           "upstream Git commit has NOT been cryptographically matched to that artifact; "
                           "crate import is reviewed/manual, not automatic discovery; "
                           "limited target contracts are not semantic equivalence proof.",
        }, sort_keys=True), flush=True)
        print("PASS reused external code from independent upstream: "
              "3/3 verified; negative controls fail-closed", flush=True)


if __name__ == "__main__":
    main()
