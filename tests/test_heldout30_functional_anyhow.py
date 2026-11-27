"""Behavioral trial: reuse an existing upstream algorithm in held-out dtolnay/anyhow.

A new target from the frozen 30-repo cohort, not a new implementation.
This is one supported case, NOT 30 functional successes or general compatibility.
"""
from __future__ import annotations

import json
import os
import shutil
import tempfile
from pathlib import Path

from test_pinned_public_multifile import CACHE, clone_exact, run
from test_unpublished_git_public import (
    UPSTREAM_URL, UPSTREAM_SHA, PACKAGE, GIT_CACHE, cli
)

COHORT = Path(__file__).with_name("heldout30_pins.json")
TARGET = "dtolnay/anyhow"
# Explicit opt-in only: full all-targets is always the default.
CONTRACT_FOCUSED = False
PROVE_FULL_TARGET_FAILURE = False
EXPECT_WORKSPACE = False
EXPLICIT_CONTRACT_TEST = False
ALLOW_BINARY_LIBRARY_ADAPTER = False
TARGET_MEMBER_DIR = None
TARGET_MEMBER_PACKAGE = None
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
    manifest = prefetch / TARGET_MEMBER_DIR / "Cargo.toml" if TARGET_MEMBER_DIR else prefetch / "Cargo.toml"
    text = manifest.read_text(encoding="utf-8")
    assert "[package]" in text
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
    # Preserve target-owned frozen dependency versions. Cargo incrementally
    # updates its existing lockfile for the newly reviewed Git capability;
    # deleting the lockfile unexpectedly upgrades upstream dependencies and
    # can break unrelated tests (observed: Tokio 1.32 -> 1.54 in mini-redis).
    pinned_lock = prefetch / "Cargo.lock"
    original_lock_bytes = pinned_lock.read_bytes() if pinned_lock.exists() else None
    fetch_args = (
        "cargo", "fetch", "--manifest-path", str(manifest),
        "--target", "x86_64-unknown-linux-gnu"
    )
    first_fetch = run(*fetch_args, timeout=240, require_success=False)
    if first_fetch.returncode != 0:
        # Pinpointed incompatibility: mini-redis locks libc 0.2.153, whereas
        # the reviewed upstream source requires >=0.2.154 via getrandom.
        # Use Cargo's official targeted solver to update ONLY the conflicting
        # library. Preserve the original pinned Tokio version and run all tests.
        if not (
            TARGET == "tokio-rs/mini-redis"
            and original_lock_bytes is not None
            and "failed to select a version for `libc`" in first_fetch.stderr
            and "previously selected package `libc v0.2.153`" in first_fetch.stderr
        ):
            raise RuntimeError("Cargo fetch failed without an approved exact conflict: "
                               + first_fetch.stderr[-3200:])
        run("cargo", "update", "--manifest-path", str(manifest),
            "--package", "libc", "--precise", "0.2.191", timeout=240)
        run(*fetch_args, timeout=240)
        print("COMPOSER_DEPENDENCY_RECOVERY: libc 0.2.153 -> 0.2.191 via Cargo", flush=True)
    lock = prefetch / "Cargo.lock"
    assert lock.is_file() and CACHE.is_dir() and GIT_CACHE.is_dir()
    if TARGET == "tokio-rs/mini-redis":
        import tomllib
        assert original_lock_bytes is not None, "mini-redis original lockfile missing"
        old_versions = {
            item["name"]: item["version"]
            for item in tomllib.loads(original_lock_bytes.decode())["package"]
            if item["name"] == "tokio"
        }
        updated_versions = {
            item["name"]: item["version"]
            for item in tomllib.loads(lock.read_text())["package"]
            if item["name"] == "tokio"
        }
        assert old_versions == updated_versions, (
            "mini-redis upstream Tokio version unexpectedly changed",
            old_versions, updated_versions
        )
        print("PASS original pinned Tokio dependency version preserved", flush=True)
    return lock


def target_copy(original: Path, base: Path, label: str, lock: Path):
    working = base / ("target-" + label)
    # The Rustls public certificate fixture importer verifies its exact Git
    # commit and tracked test-ca tree; retain only this CI-created public
    # checkout's .git metadata for that verification. Composer's published
    # output must still never contain a .git directory.
    ignored = None if TARGET == "rustls/rustls" else shutil.ignore_patterns(".git")
    shutil.copytree(original, working, ignore=ignored)
    member_root = working / TARGET_MEMBER_DIR if TARGET_MEMBER_DIR else working
    (member_root / "tests").mkdir(exist_ok=True)
    (member_root / CONTRACT).write_text(INDEPENDENT_ORACLE, encoding="utf-8")
    if EXPLICIT_CONTRACT_TEST:
        manifest = member_root / "Cargo.toml"
        with manifest.open("a", encoding="utf-8") as file:
            file.write("\n[[test]]\nname = 'composer_heldout30_totient_contract'\npath = 'tests/composer_heldout30_totient_contract.rs'\n")
    shutil.copy2(lock, working / "Cargo.lock")
    contract_path = str(Path(TARGET_MEMBER_DIR) / CONTRACT) if TARGET_MEMBER_DIR else CONTRACT
    originals = {name:(working/name).read_bytes() for name in (
        "Cargo.toml","Cargo.lock",contract_path
    )}
    if TARGET_MEMBER_DIR:
        originals[str(Path(TARGET_MEMBER_DIR) / "Cargo.toml")] = (member_root/"Cargo.toml").read_bytes()
    if (member_root/"src/lib.rs").exists():
        relative_lib = str(Path(TARGET_MEMBER_DIR) / "src/lib.rs") if TARGET_MEMBER_DIR else "src/lib.rs"
        originals[relative_lib] = (member_root/"src/lib.rs").read_bytes()
    else:
        originals["src/main.rs"] = (working/"src/main.rs").read_bytes()
    return working, originals


def check_unchanged(working: Path, originals: dict):
    for name, previous in originals.items():
        assert (working/name).read_bytes() == previous, "original modified: " + name


def verify(working: Path, upstream: Path, base: Path, name: str,
           public_symbol: str, require_success=True, contract_focused=False):
    output = base / ("output-" + name)
    arguments = [
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
    ]
    if contract_focused:
        arguments.append("--contract-focused")
    if ALLOW_BINARY_LIBRARY_ADAPTER:
        arguments.append("--allow-binary-library-adapter")
    if TARGET_MEMBER_DIR:
        arguments.extend(["--target-member-dir", TARGET_MEMBER_DIR])
    response = cli(*arguments, require_success=require_success)
    return response, output


def main():
    cases = json.loads(COHORT.read_text(encoding="utf-8"))["repos"]
    pinned = next(entry["sha"] for entry in cases if entry["repo"] == TARGET)
    images = {
        "nightly-2026-11-25": "docker.io/rustlang/rust@sha256:eb59a92b7f3abf22b62141998ad2f2b477425ccd2f796a1dd87e79bc78beccab",
        "stable-system-libs": "docker.io/library/rust@sha256:8a4e90c7946b74626488b1446f712c0fe0ed1a596cbd3d871684bc9c5433ae07",
        "nightly-gungraun-0.20.0": "composer-verifier-gungraun:0.20.0",
        "nightly-rustc-dev-2026-11-25": "composer-verifier-rustc-dev:nightly-2026-11-25",
    }
    image = images.get(os.getenv("COMPOSER_VERIFIER_TOOLCHAIN"), "rust:1.99-slim")
    run("docker", "image", "inspect", image)
    with tempfile.TemporaryDirectory(prefix="composer-heldout30-behavioral-") as tmp:
        base = Path(tmp)
        upstream = base / "thealgorithms-reviewed"
        target = base / "anyhow-heldout"
        clone_exact(UPSTREAM_URL, UPSTREAM_SHA, upstream)
        clone_exact(f"https://github.com/{TARGET}.git", pinned, target)
        assert (upstream / "LICENSE").is_file()
        if ALLOW_BINARY_LIBRARY_ADAPTER:
            assert (target / "src/main.rs").is_file()
            assert not (target / "src/lib.rs").exists()
        elif TARGET_MEMBER_DIR:
            assert (target / TARGET_MEMBER_DIR / "src/lib.rs").is_file()
        else:
            assert (target / "src/lib.rs").is_file()
        assert any((target / name).is_file() for name in ("LICENSE-MIT","LICENSE-APACHE","LICENSE"))
        lock = prefetch_cargo(target, base)
        if PROVE_FULL_TARGET_FAILURE:
            # Preserve the real default failure; never auto-downgrade tests.
            rejected, unchanged = target_copy(target, base, "full-scope-refusal", lock)
            fail, published = verify(rejected, upstream, base, "full-scope-refusal",
                                     "number_theory::euler_totient", require_success=False)
            assert fail.returncode != 0 and not published.exists()
            assert "feature(test)" in fail.stderr or "E0554" in fail.stderr, fail.stderr[-1200:]
            check_unchanged(rejected, unchanged)
            print(f"EXPECTED_REFUSAL {TARGET}: stable Rust rejects upstream nightly benchmark in default all-targets mode", flush=True)
        expected_scope = ("selected_workspace_member_all_targets" if TARGET_MEMBER_DIR else
                          "library_and_integration_tests" if CONTRACT_FOCUSED else "all_targets")
        positives = []
        for index in range(3):
            label = f"positive-{index}"
            working, originals = target_copy(target, base, label, lock)
            response, output = verify(working, upstream, base, label,
                                       "number_theory::euler_totient",
                                       contract_focused=CONTRACT_FOCUSED)
            proof = json.loads(response.stdout)
            assert proof["offline_tests_passed"] is True
            assert proof["verification_scope"] == expected_scope
            assert proof["source_git_objects_verified"] is True
            assert proof["lockfile_git_commit_verified"] is True
            assert proof["source_checkout_commit_verified"] is True
            assert (output / "Cargo.lock").is_file()
            assert not (output / ".git").exists(), "private Git metadata published"
            assert UPSTREAM_SHA in (output / "Cargo.lock").read_text()
            evidence = json.loads((output / "composer-upstream-git-evidence.json").read_text())
            assert evidence["network_disabled"] and evidence["offline_tests_passed"]
            assert evidence["docker_image_reference"] == image
            assert evidence["docker_cpu_limit"] == int(os.getenv("COMPOSER_VERIFIER_CPUS", "1"))
            assert evidence["docker_pids_limit"] == int(os.getenv("COMPOSER_VERIFIER_PIDS", "64"))
            assert evidence["docker_timeout_seconds"] == int(os.getenv("COMPOSER_VERIFIER_TIMEOUT_SECS", "240"))
            assert evidence["docker_all_features"] is (os.getenv("COMPOSER_VERIFIER_FEATURES", "default") == "all")
            assert evidence["docker_rustflags_profile"] == os.getenv("COMPOSER_VERIFIER_RUSTFLAGS_PROFILE", "default")
            assert evidence["cargo_workspace_root_package"] is EXPECT_WORKSPACE
            assert evidence["binary_library_adapter_created"] is ALLOW_BINARY_LIBRARY_ADAPTER
            assert evidence["root_virtual_workspace"] is bool(TARGET_MEMBER_DIR)
            assert evidence["target_member_dir"] == TARGET_MEMBER_DIR
            assert evidence["target_member_package"] == TARGET_MEMBER_PACKAGE
            assert evidence["reviewed_public_test_fixture_tree_oid"] == (
                "b42c258742023dde0c447979ee72349f92c2006c"
                if TARGET == "rustls/rustls" else None
            )
            assert evidence["verification_scope"] == expected_scope
            assert evidence["docker_memory_limit_mb"] == int(os.getenv("COMPOSER_VERIFIER_MEMORY_MB", "1024"))
            assert evidence["third_party_benchmarks_verified"] is (not CONTRACT_FOCUSED)
            # Some real projects emit enough Cargo tree output to truncate
            # the success summary in the bounded log excerpt. The verifier
            # already requires cargo test --all-targets to exit 0 before it
            # sets offline_tests_passed; retain evidence, not a brittle phrase.
            assert evidence["log_tail"].strip(), "missing bounded verifier log"
            contract_path = Path(TARGET_MEMBER_DIR) / CONTRACT if TARGET_MEMBER_DIR else Path(CONTRACT)
            assert (output / contract_path).read_bytes() == (working / contract_path).read_bytes()
            check_unchanged(working, originals)
            positives.append({"verified": True, "elapsed_seconds": proof["elapsed_seconds"]})
            print(f"PASS held-out target {TARGET} functional trial {index+1}/3",flush=True)

        wrong, originals = target_copy(target, base, "wrong-factorial", lock)
        failed, output = verify(wrong, upstream, base, "wrong-factorial",
                                "math::factorial::factorial", require_success=False,
                                contract_focused=CONTRACT_FOCUSED)
        assert failed.returncode != 0, "wrong but compatible behavior was accepted"
        assert not output.exists(), "unverified target was published"
        check_unchanged(wrong, originals)
        print("PASS wrong-function control rejected, no publication", flush=True)
        print("EVIDENCE_JSON " + json.dumps({
            "target": TARGET, "pinned_target_commit": pinned,
            "upstream": UPSTREAM_URL, "pinned_upstream_commit": UPSTREAM_SHA,
            "successful_independent_trials": positives,
            "negative_behavior_rejected": True,
            "verification_scope": expected_scope,
            "default_full_target_scope_rejected": PROVE_FULL_TARGET_FAILURE,
            "scope": "one held-out Rust target with one reviewed Git capability",
            "other_heldout_targets_not_tested_in_this_run": 29,
            "production_ready": False,
        },sort_keys=True),flush=True)


if __name__ == "__main__":
    main()
