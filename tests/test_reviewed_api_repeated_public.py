"""Repeat accepted reviewed API suggestions against pinned existing Rust projects.

Source and target are cloned to exact revisions. Source execution occurs only in
Composer's restricted, offline Docker verification. Every iteration copies a
fresh original target; no previous integrated output is reused as input.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import tempfile
from time import perf_counter

from test_pinned_public_multifile import (
    CACHE, GEOMETRY_REPO, GEOMETRY_SHA, SOURCE_REPO, SOURCE_SHA,
    cli, clone_exact, make_capsule, prepare_registry, run,
)

HASH_REPO = "https://github.com/rust-lang/rustc-hash.git"
HASH_SHA = "fdfe9e6ebea0f0bd3de95d5af849f5d03f4b7fbb"
REPEATS = int(os.environ.get("COMPOSER_REVIEWED_REPEATS", "3"))


def case(
    root: Path, iteration: int, label: str, source: Path, capsule: Path,
    target: Path, target_package: str, stub: str, contract: str,
    target_function: str, expected_rule: str,
) -> dict:
    work = root / f"target-{label}-{iteration}"
    shutil.copytree(target, work, ignore=shutil.ignore_patterns(".git"))
    library = work / "src/lib.rs"
    original_lib = library.read_bytes()
    cargo_original = (work / "Cargo.toml").read_bytes()
    library.write_bytes(original_lib + b"\n" + stub.encode("utf-8"))
    with_stub = library.read_bytes()
    (work / "tests").mkdir(exist_ok=True)
    test = work / "tests/reviewed_contract.rs"
    test.write_text(contract, encoding="utf-8")
    original_contract = test.read_bytes()
    begin = perf_counter()
    suggestion = json.loads(cli(
        "suggest-rust-api", str(capsule), str(work)
    ).stdout)
    matched = [req for req in suggestion["requirements"]
               if req["target_function"] == target_function]
    assert len(matched) == 1 and matched[0]["best_suggestion"], (
        label, iteration, matched
    )
    exact_rule = matched[0]["best_suggestion"]["suggested_cli_rule"]
    assert exact_rule == expected_rule, (exact_rule, expected_rule)
    discovered_ms = round((perf_counter() - begin) * 1000, 1)

    output = root / f"verified-{label}-{iteration}"
    start_verification = perf_counter()
    payload = json.loads(cli(
        "verify-rust", str(capsule), str(work),
        "--output", str(output), "--license-reviewed",
        "--registry-cache", str(CACHE),
        "--reviewed-api-rule", exact_rule,
        "--contract-test", "tests/reviewed_contract.rs",
    ).stdout)
    verify_seconds = round(perf_counter() - start_verification, 2)
    assert payload["isolated_tests_passed"] is True
    assert payload["reviewed_api_rule"] == exact_rule
    assert payload["reviewed_contract_test"] == "tests/reviewed_contract.rs"
    evidence = json.loads((output / "composer-build-evidence.json").read_text())
    assert evidence["tests_passed"] is True
    assert evidence["network_disabled"] is True
    assert evidence["reviewed_api_rule"] == exact_rule
    assert evidence["reviewed_contract_test"] == "tests/reviewed_contract.rs"
    assert "test result: ok" in evidence["log_tail"], "No passing Cargo test result"
    published_lib = (output / "src/lib.rs").read_text()
    assert published_lib.count(f"pub fn {target_function}") == 1
    assert "todo!()" not in published_lib.split(
        f"pub fn {target_function}", 1
    )[1].split("}", 1)[0]
    assert library.read_bytes() == with_stub
    assert (work / "Cargo.toml").read_bytes() == cargo_original
    assert test.read_bytes() == original_contract
    assert (target / "src/lib.rs").read_bytes() == original_lib
    assert (output / "src/lib.rs").is_file()
    print(
        f"PASS case={label} repeat={iteration}/{REPEATS} "
        f"discovery_ms={discovered_ms} verification_s={verify_seconds}",
        flush=True,
    )
    return {
        "case": label,
        "run": iteration,
        "status": "verified",
        "discovery_ms": discovered_ms,
        "verification_seconds": verify_seconds,
        "offline_compilation_and_contract_tests": True,
        "original_target_unchanged": True,
    }


def rejection_check(root: Path, source: Path, capsule: Path, target: Path) -> None:
    work = root / "negative-target"
    shutil.copytree(target, work, ignore=shutil.ignore_patterns(".git"))
    original = (work / "src/lib.rs").read_bytes()
    (work / "src/lib.rs").write_bytes(
        original + b"\npub fn adapted_totient(n: u32) -> u128 { todo!() }\n"
    )
    (work / "tests").mkdir(exist_ok=True)
    (work / "tests/reviewed_contract.rs").write_text(
        "use contest_algorithms::adapted_totient;\n"
        "#[test] fn asserts() { assert_eq!(adapted_totient(9_u32), 6_u128); }\n",
        encoding="utf-8",
    )
    unchanged = (work / "src/lib.rs").read_bytes()
    for index, (rule, test) in enumerate([
        ("euler_totient=adapted_totient(u64)->u128", "tests/reviewed_contract.rs"),
        ("euler_totient=adapted_totient(u32)->u128", "tests/missing.rs"),
        ("euler_totient=adapted_totient(u32)->u128", "../leak.rs"),
        ("euler_totient=adapted_totient(u32)->u128", "tests/../../leak.rs"),
    ]):
        out = root / f"rejected-{index}"
        response = cli(
            "verify-rust", str(capsule), str(work), "--output", str(out),
            "--license-reviewed", "--registry-cache", str(CACHE),
            "--reviewed-api-rule", rule, "--contract-test", test,
            require_success=False,
        )
        assert response.returncode != 0, (rule, test)
        assert not out.exists(), "Invalid contract published output"
        assert (work / "src/lib.rs").read_bytes() == unchanged
    # False-completion probe: a deliberately wrong independent assertion
    # must make the real offline Docker run fail and leave no output.
    failing = root / "negative-incorrect-behavior"
    shutil.copytree(work, failing, ignore=shutil.ignore_patterns(".git"))
    failing_test = failing / "tests/reviewed_contract.rs"
    failing_test.write_text(
        "use contest_algorithms::adapted_totient;\n"
        "#[test] fn rejects_wrong_answer() { "
        "assert_eq!(adapted_totient(9_u32), 999_u128); }\n",
        encoding="utf-8",
    )
    invalid_output = root / "must-not-publish-false-completion"
    observed = cli(
        "verify-rust", str(capsule), str(failing),
        "--output", str(invalid_output), "--license-reviewed",
        "--registry-cache", str(CACHE),
        "--reviewed-api-rule", "euler_totient=adapted_totient(u32)->u128",
        "--contract-test", "tests/reviewed_contract.rs",
        require_success=False,
    )
    assert observed.returncode != 0, "Incorrect target behavior must fail"
    assert not invalid_output.exists(), "Incorrect contract must never publish"
    assert (work / "src/lib.rs").read_bytes() == unchanged
    print("PASS false-completion probe: deliberately wrong contract failed Docker verification", flush=True)
    print("PASS negative reviewed execution: stale rule, missing contract and traversal rejected", flush=True)


def main() -> None:
    assert REPEATS == 3, "Acceptance requires exactly 3 independent attempts per case"
    run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-repeated-reviewed-") as temp:
        root = Path(temp)
        algo = root / "source-algorithms"
        ebtech = root / "source-ebtech"
        hash_target = root / "target-rustc-hash"
        clone_exact(GEOMETRY_REPO, GEOMETRY_SHA, algo)
        clone_exact(SOURCE_REPO, SOURCE_SHA, ebtech)
        clone_exact(HASH_REPO, HASH_SHA, hash_target)
        assert (hash_target / "LICENSE-MIT").is_file()
        totient = make_capsule(
            algo, root, "euler_totient",
            "src/number_theory/euler_totient.rs", "reviewed-totient",
            {"src/number_theory/euler_totient.rs"},
            expected_sha=GEOMETRY_SHA,
        )
        cube = make_capsule(
            algo, root, "perfect_cube_binary_search",
            "src/math/perfect_cube.rs", "reviewed-cube",
            {"src/math/perfect_cube.rs"}, expected_sha=GEOMETRY_SHA,
        )
        assert prepare_registry(hash_target, root, "rustc-hash"), "rustc-hash offline registry cache preparation failed"
        assert prepare_registry(ebtech, root, "ebtech"), "EbTech registry cache preparation failed"
        rejection_check(root, algo, totient, ebtech)
        outcomes = []
        for iteration in range(1, REPEATS + 1):
            outcomes.append(case(
                root, iteration, "totient-ebtech", algo, totient, ebtech,
                "contest_algorithms",
                "pub fn adapted_totient(n: u32) -> u128 { todo!() }\n",
                "use contest_algorithms::adapted_totient;\n"
                "#[test] fn matches_number_theory_contract() {\n"
                "assert_eq!(adapted_totient(9_u32), 6_u128);\n"
                "assert_eq!(adapted_totient(10_u32), 4_u128);\n"
                "assert_eq!(adapted_totient(13_u32), 12_u128);\n}\n",
                "adapted_totient", "euler_totient=adapted_totient(u32)->u128",
            ))
            outcomes.append(case(
                root, iteration, "cube-rustc-hash", algo, cube, hash_target, "rustc_hash",
                "pub fn perfect_cube_binary_search_checked(n: i32) -> bool { unimplemented!() }\n",
                "use rustc_hash::perfect_cube_binary_search_checked;\n"
                "#[test] fn matches_perfect_cube_contract() {\n"
                "assert!(perfect_cube_binary_search_checked(27_i32));\n"
                "assert!(perfect_cube_binary_search_checked(64_i32));\n"
                "assert!(!perfect_cube_binary_search_checked(28_i32));\n"
                "assert!(perfect_cube_binary_search_checked(-8_i32));\n}\n",
                "perfect_cube_binary_search_checked",
                "perfect_cube_binary_search=perfect_cube_binary_search_checked(i32)->bool",
            ))
        assert len(outcomes) == 6
        assert all(x["status"] == "verified" for x in outcomes)
        print("EVIDENCE_JSON " + json.dumps({
            "schema_version": 1,
            "total_attempts": len(outcomes),
            "successful_attempts": len(outcomes),
            "cases": outcomes,
            "limitations": "Two curated source functions from one pinned public source repo, two unrelated existing public targets including no_std crate; contract tests supplied manually; no general reliability inference.",
        }, sort_keys=True), flush=True)
        print("PASS 6/6 reviewed suggestion -> offline Docker compilation -> "
              "independent target contract checks on two existing public targets",
              flush=True)


if __name__ == "__main__":
    main()
