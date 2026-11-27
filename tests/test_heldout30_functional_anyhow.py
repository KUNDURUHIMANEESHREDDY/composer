"""Behavioral trial: reuse an existing upstream algorithm in held-out dtolnay/anyhow.

A new target from the frozen 30-repo cohort, not a new implementation.
This is one supported case, NOT 30 functional successes or general compatibility.
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

COHORT = Path(__file__).with_name("heldout30_pins.json")
TARGET = "dtolnay/anyhow"
CONTRACT = "tests/composer_heldout30_totient_contract.rs"
INDEPENDENT_ORACLE = """use anyhow::ported_totient;
#[test]
fn totient_behavior_is_not_merely_type_compatible() {
    for (n, expected) in [(1, 1), (2, 1), (3, 2), (9, 6),
                          (10, 4), (13, 12), (36, 12), (100, 40)] {
        assert_eq!(ported_totient(n), expected, "totient({n})");
    }
}
"""


def prefetch_cargo(target: Path, base: Path) -> Path:
    prefetch = base / "prefetch"
    shutil.copytree(target, prefetch, ignore=shutil.ignore_patterns(".git"))
    manifest = prefetch / "Cargo.toml"
    text = manifest.read_text(encoding="utf-8")
    assert "[package]" in text and "[workspace]" not in text
    assert "composer_git_the_algorithms_rust" not in text
    dependency = (
        f'composer_git_the_algorithms_rust = {{ package = "{PACKAGE}", '
        f'git = "{UPSTREAM_URL}", rev = "{UPSTREAM_SHA}" }}'
    )
    if "[dependencies]" in text:
        text = text.replace("[dependencies]", "[dependencies]\n" + dependency, 1)
    else:
        text += "\n[dependencies]\n" + dependency + "\n"
    manifest.write_text(text, encoding="utf-8")
    # Use Cargo's own dependency solver only in a disposable prefetch copy.
    stale = prefetch / "Cargo.lock"
    if stale.exists():
        stale.unlink()
    run("cargo", "fetch", "--manifest-path", str(manifest),
        "--target", "x86_64-unknown-linux-gnu", timeout=240)
    lock = prefetch / "Cargo.lock"
    assert lock.is_file() and CACHE.is_dir() and GIT_CACHE.is_dir()
    return lock


def target_copy(original: Path, base: Path, label: str, lock: Path):
    working = base / ("target-" + label)
    shutil.copytree(original, working, ignore=shutil.ignore_patterns(".git"))
    (working / "tests").mkdir(exist_ok=True)
    (working / CONTRACT).write_text(INDEPENDENT_ORACLE, encoding="utf-8")
    shutil.copy2(lock, working / "Cargo.lock")
    originals = {name:(working/name).read_bytes() for name in (
        "Cargo.toml","Cargo.lock","src/lib.rs",CONTRACT
    )}
    return working, originals


def check_unchanged(working: Path, originals: dict):
    for name, previous in originals.items():
        assert (working/name).read_bytes() == previous, "original modified: " + name


def verify(working: Path, upstream: Path, base: Path, name: str,
           public_symbol: str, require_success=True):
    output = base / ("output-" + name)
    response = cli(
        "verify-external-git-rust", str(working),
        "--output", str(output),
        "--upstream-checkout", str(upstream),
        "--upstream-repository", UPSTREAM_URL.removesuffix(".git"),
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
    return response, output


def main():
    cases = json.loads(COHORT.read_text(encoding="utf-8"))["repos"]
    pinned = next(entry["sha"] for entry in cases if entry["repo"] == TARGET)
    run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-heldout30-behavioral-") as tmp:
        base = Path(tmp)
        upstream = base / "thealgorithms-reviewed"
        target = base / "anyhow-heldout"
        clone_exact(UPSTREAM_URL, UPSTREAM_SHA, upstream)
        clone_exact(f"https://github.com/{TARGET}.git", pinned, target)
        assert (upstream / "LICENSE").is_file()
        assert (target / "src/lib.rs").is_file()
        assert any((target / name).is_file() for name in ("LICENSE-MIT","LICENSE-APACHE","LICENSE"))
        lock = prefetch_cargo(target, base)
        positives = []
        for index in range(3):
            label = f"positive-{index}"
            working, originals = target_copy(target, base, label, lock)
            response, output = verify(working, upstream, base, label,
                                       "number_theory::euler_totient")
            proof = json.loads(response.stdout)
            assert proof["offline_tests_passed"] is True
            assert proof["verification_scope"] == "all_targets"
            assert proof["source_git_objects_verified"] is True
            assert proof["lockfile_git_commit_verified"] is True
            assert proof["source_checkout_commit_verified"] is True
            assert (output / "Cargo.lock").is_file()
            assert UPSTREAM_SHA in (output / "Cargo.lock").read_text()
            evidence = json.loads((output / "composer-upstream-git-evidence.json").read_text())
            assert evidence["network_disabled"] and evidence["offline_tests_passed"]
            assert evidence["verification_scope"] == "all_targets"
            assert "test result: ok" in evidence["log_tail"]
            check_unchanged(working, originals)
            positives.append({"verified": True, "elapsed_seconds": proof["elapsed_seconds"]})
            print(f"PASS held-out target {TARGET} functional trial {index+1}/3",flush=True)

        wrong, originals = target_copy(target, base, "wrong-factorial", lock)
        failed, output = verify(wrong, upstream, base, "wrong-factorial",
                                "math::factorial::factorial", require_success=False)
        assert failed.returncode != 0, "wrong but compatible behavior was accepted"
        assert not output.exists(), "unverified target was published"
        check_unchanged(wrong, originals)
        print("PASS wrong-function control rejected, no publication", flush=True)
        print("EVIDENCE_JSON " + json.dumps({
            "target": TARGET, "pinned_target_commit": pinned,
            "upstream": UPSTREAM_URL, "pinned_upstream_commit": UPSTREAM_SHA,
            "successful_independent_trials": positives,
            "negative_behavior_rejected": True,
            "scope": "one held-out Rust target with one reviewed Git capability",
            "remaining_heldout_functional_targets": 29,
            "production_ready": False,
        },sort_keys=True),flush=True)


if __name__ == "__main__":
    main()
