"""Held-out public-repository semantic benchmark for reviewed Rust integration.

The public repositories were not used in the earlier 6/6 curated benchmark.
All checkouts are immutable, targets are disposable, and all third-party
compilation/test execution is confined to Composer's offline Docker verifier.
These are engineer-authored tests, not automatically inferred requirements.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import tempfile
from time import perf_counter

from test_pinned_public_multifile import (
    CACHE, cli, clone_exact, make_capsule, prepare_registry, run,
)

STRSIM_REPO = "https://github.com/rapidfuzz/strsim-rs.git"
STRSIM_SHA = "dacc84c0dc61eff0ee0ff66962bcf2e17018ad26"
TRIGRAM_REPO = "https://github.com/ijt/trigram.git"
TRIGRAM_SHA = "45af254dcd7b5c8aac4feb336192204791cd1e2f"
REPEATS = int(os.environ.get("COMPOSER_UNSEEN_REPEATS", "3"))

JARO_STUB = "pub fn jaro(a: &str, b: &str) -> f64 { todo!() }\n"
JARO_TEST = (
    "use trigram::jaro;\n"
    "#[test] fn jaro_independent_edges() {\n"
    "  assert_eq!(jaro(\"\", \"\"), 1.0);\n"
    "  assert_eq!(jaro(\"abc\", \"xyz\"), 0.0);\n"
    "  assert!((jaro(\"MARTHA\", \"MARHTA\") - 0.9444444444).abs() < 0.00001);\n"
    "}\n"
)
TRIGRAM_STUB = "pub fn similarity(a: &str, b: &str) -> f64 { todo!() }\n"
TRIGRAM_TEST = (
    "use strsim::similarity;\n"
    "#[test] fn trigram_independent_edges() {\n"
    "  assert_eq!(similarity(\"foo\", \"food\"), 0.5);\n"
    "  assert_eq!(similarity(\"same, but different?\", \"same but different\"), 1.0);\n"
    "}\n"
)


def stage_target(root: Path, target: Path, label: str, stub: str, contract: str) -> Path:
    work = root / label
    shutil.copytree(target, work, ignore=shutil.ignore_patterns(".git"))
    lib = work / "src/lib.rs"
    lib.write_bytes(lib.read_bytes() + b"\n" + stub.encode("utf-8"))
    tests = work / "tests"
    tests.mkdir(exist_ok=True)
    (tests / "reviewed_contract.rs").write_text(contract, encoding="utf-8")
    return work


def matching_rule(capsule: Path, target: Path, function: str, expected: str) -> str:
    original = (target / "src/lib.rs").read_bytes()
    report = json.loads(cli("suggest-rust-api", str(capsule), str(target)).stdout)
    assert (target / "src/lib.rs").read_bytes() == original, "Discovery modified target"
    requirements = [x for x in report["requirements"] if x["target_function"] == function]
    assert len(requirements) == 1, (function, requirements)
    candidate = requirements[0]["best_suggestion"]
    assert candidate, f"No unambiguous reviewed candidate for {function}"
    rule = candidate["suggested_cli_rule"]
    assert rule == expected, (rule, expected)
    return rule


def verify(root: Path, capsule: Path, target: Path, label: str, rule: str,
           expect_success: bool) -> dict:
    output = root / f"published-{label}"
    original = {
        name: (target / name).read_bytes()
        for name in ("Cargo.toml", "src/lib.rs", "tests/reviewed_contract.rs")
    }
    started = perf_counter()
    proc = cli(
        "verify-rust", str(capsule), str(target), "--output", str(output),
        "--license-reviewed", "--registry-cache", str(CACHE),
        "--reviewed-api-rule", rule,
        "--contract-test", "tests/reviewed_contract.rs",
        require_success=False,
    )
    seconds = round(perf_counter() - started, 2)
    for name, before in original.items():
        assert (target / name).read_bytes() == before, f"{label} mutated input {name}"
    if proc.returncode:
        assert not output.exists(), f"{label}: published failed verification"
        print(f"REJECTED {label} after {seconds}s: {proc.stderr[-1500:]}", flush=True)
        assert not expect_success, f"{label}: expected verified integration, got rejection"
        return {"case": label, "status": "rejected", "seconds": seconds,
                "reason_tail": proc.stderr[-600:]}
    assert expect_success, f"{label}: unexpected publication"
    evidence = json.loads((output / "composer-build-evidence.json").read_text())
    assert evidence["tests_passed"] is True
    assert evidence["network_disabled"] is True
    assert evidence["reviewed_api_rule"] == rule
    assert evidence["reviewed_contract_test"] == "tests/reviewed_contract.rs"
    assert "test result: ok" in evidence["log_tail"]
    assert (output / "src/lib.rs").is_file()
    assert (output / "composer-source-attributions.json").is_file()
    assert (output / "third_party_licenses").is_dir()
    assert (output / "LICENSE").is_file(), "Original target license lost"
    print(f"VERIFIED {label} after {seconds}s (isolated contract and full suite)", flush=True)
    return {"case": label, "status": "verified", "seconds": seconds}


def discovery_negatives(root: Path, source_capsule: Path, target: Path) -> None:
    misleading = stage_target(
        root, target, "misleading-name", TRIGRAM_STUB,
        "use strsim::similarity;\n"
        "#[test] fn test() { assert_eq!(similarity(\"a\", \"a\"), 1.0); }\n",
    )
    report = json.loads(cli("suggest-rust-api", str(source_capsule),
                            str(misleading)).stdout)
    candidate = next(x for x in report["requirements"]
                     if x["target_function"] == "similarity")["best_suggestion"]
    assert candidate is None, "Invented semantic match for unrelated similarity API"

    invalid = stage_target(
        root, target, "incompatible-arguments",
        "pub fn jaro(a: u64, b: u64) -> f64 { todo!() }\n",
        JARO_TEST,
    )
    report = json.loads(cli("suggest-rust-api", str(source_capsule),
                            str(invalid)).stdout)
    candidate = next(x for x in report["requirements"]
                     if x["target_function"] == "jaro")["best_suggestion"]
    assert candidate is None, "Invented conversion from u64 into &str"
    print("PASS read-only misleading-name and incompatible-signature rejection",
          flush=True)


def main() -> None:
    assert REPEATS == 3, "Acceptance benchmark requires three independent attempts"
    run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-heldout-semantics-") as tmp:
        root = Path(tmp)
        strsim = root / "source-strsim"
        trigram = root / "source-trigram"
        clone_exact(STRSIM_REPO, STRSIM_SHA, strsim)
        clone_exact(TRIGRAM_REPO, TRIGRAM_SHA, trigram)
        assert (strsim / "LICENSE").is_file()
        assert (trigram / "LICENSE").is_file()
        jaro_capsule = make_capsule(
            strsim, root, "jaro", "src/lib.rs", "heldout-jaro",
            {"src/lib.rs"}, expected_sha=STRSIM_SHA, license_file="LICENSE",
        )
        trigram_capsule = make_capsule(
            trigram, root, "similarity", "src/lib.rs", "heldout-trigram",
            {"src/lib.rs"}, expected_sha=TRIGRAM_SHA, license_file="LICENSE",
        )
        assert prepare_registry(trigram, root, "trigram"), "Missing offline trigram registry cache"
        assert prepare_registry(strsim, root, "strsim"), "Missing offline strsim registry cache"

        discovery_negatives(root, jaro_capsule, strsim)
        rows = []
        for index in range(1, REPEATS + 1):
            # Held-out public source A -> held-out public target B.
            work = stage_target(root, trigram, f"jaro-attempt-{index}",
                                JARO_STUB, JARO_TEST)
            rule = matching_rule(jaro_capsule, work, "jaro", "jaro=jaro(&str,&str)->f64")
            rows.append(verify(root, jaro_capsule, work, f"jaro-attempt-{index}",
                               rule, expect_success=True))

        # Wrong-but-compiling behavior: Jaro is not trigram/Jaccard similarity.
        # A correct compiler result must not be treated as contract success.
        wrong = stage_target(
            root, trigram, "wrong-semantic-target", JARO_STUB,
            "use trigram::jaro;\n"
            "#[test] fn detects_semantic_mismatch() {\n"
            "  assert_eq!(jaro(\"foo\", \"food\"), 0.5_f64);\n"
            "}\n",
        )
        rows.append(verify(root, jaro_capsule, wrong, "wrong-semantic-target",
                           "jaro=jaro(&str,&str)->f64", expect_success=False))

        # A different public-source dependency graph is tested in the reverse
        # direction. This is an exploratory boundary; do not count a refusal
        # as a completed integration.
        reverse = stage_target(root, strsim, "reverse-trigram-source",
                               TRIGRAM_STUB, TRIGRAM_TEST)
        reverse_rule = matching_rule(
            trigram_capsule, reverse, "similarity",
            "similarity=similarity(&str,&str)->f64",
        )
        output = root / "published-reverse-trigram-source"
        original = (reverse / "src/lib.rs").read_bytes()
        proc = cli(
            "verify-rust", str(trigram_capsule), str(reverse),
            "--output", str(output), "--license-reviewed",
            "--registry-cache", str(CACHE),
            "--reviewed-api-rule", reverse_rule,
            "--contract-test", "tests/reviewed_contract.rs",
            require_success=False,
        )
        assert (reverse / "src/lib.rs").read_bytes() == original
        if proc.returncode:
            assert not output.exists(), "Rejected reverse dependency published output"
            rows.append({"case": "reverse-dependencies", "status": "rejected",
                         "reason_tail": proc.stderr[-600:]})
            print("REJECTED reverse dependency transfer: " + proc.stderr[-1200:],
                  flush=True)
        else:
            evidence = json.loads((output / "composer-build-evidence.json").read_text())
            assert evidence["tests_passed"] and evidence["network_disabled"]
            rows.append({"case": "reverse-dependencies", "status": "verified"})
            print("VERIFIED reverse dependency transfer", flush=True)

        correct = sum(x["status"] == "verified" for x in rows if
                      x["case"].startswith("jaro-attempt-"))
        print("EVIDENCE_JSON " + json.dumps({
            "schema_version": 1,
            "heldout_source_sha": STRSIM_SHA,
            "heldout_target_sha": TRIGRAM_SHA,
            "successful_heldout_attempts": correct,
            "required_heldout_attempts": REPEATS,
            "cases": rows,
            "limitations": (
                "Reviewed, manually specified APIs and independent contracts; "
                "no autonomous adaptation proof. A rejected dependency transfer "
                "is not semantic success. Docker is not hostile-code isolation."
            ),
        }, sort_keys=True), flush=True)
        assert correct == REPEATS, f"Held-out public-to-public integration only {correct}/{REPEATS}"
        print("PASS held-out semantics with independent behavioral checks", flush=True)


if __name__ == "__main__":
    main()
