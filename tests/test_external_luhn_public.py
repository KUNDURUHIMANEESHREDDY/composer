"""Real external-source capability reuse: luhn-rs -> unrelated trigram.

No implementation is re-created. The Luhn validation algorithm is taken from
a pinned, MIT-licensed external repository. The only new target code is a
placeholder API stub and independent acceptance tests. Composer must wire the
upstream source unchanged, run its full isolated test suite, preserve license
and commit evidence, and never publish failing outputs.

Scope: human-selected source and contracts; *not* automatic capability search.
"""
from __future__ import annotations

import json
from pathlib import Path
import shutil
import tempfile
from time import perf_counter

from test_pinned_public_multifile import (
    CACHE, cli, clone_exact, make_capsule, prepare_registry, run,
)

UPSTREAM = "https://github.com/jeffcarp/luhn-rs.git"
UPSTREAM_SHA = "dbe3684f2f4b36bccfb3699f26fab9eaf17335fc"
TARGET_REPO = "https://github.com/ijt/trigram.git"
TARGET_SHA = "45af254dcd7b5c8aac4feb336192204791cd1e2f"
RULE = "valid=valid_checksum(&str)->bool"
STUB = "\npub fn valid_checksum(input: &str) -> bool { todo!() }\n"
TEST_PATH = "tests/composer_external_contract.rs"
GOOD_CONTRACT = (
    "use trigram::valid_checksum;\n"
    "#[test] fn checks_known_valid_and_invalid_values() {\n"
    'assert!(valid_checksum("79927398713"));\n'
    'assert!(valid_checksum("4111-1111-1111-1111"));\n'
    'assert!(!valid_checksum("79927398710"));\n'
    'assert!(!valid_checksum("banana"));\n'
    'assert!(!valid_checksum(""));\n'
    "}\n"
)
WRONG_CONTRACT = (
    "use trigram::valid_checksum;\n"
    "#[test] fn intentionally_incorrect_semantics_must_fail() {\n"
    'assert!(valid_checksum("banana"));\n'
    "}\n"
)


def target_copy(root: Path, original: Path, label: str, contract: str) -> Path:
    dest = root / label
    shutil.copytree(original, dest, ignore=shutil.ignore_patterns(".git"))
    library = dest / "src/lib.rs"
    library.write_text(library.read_text(encoding="utf-8") + STUB, encoding="utf-8")
    (dest / "tests").mkdir(exist_ok=True)
    (dest / TEST_PATH).write_text(contract, encoding="utf-8")
    return dest


def snapshots(target: Path) -> dict[str, bytes]:
    return {path: (target / path).read_bytes()
            for path in ("src/lib.rs", "Cargo.toml", TEST_PATH)}


def run_case(root: Path, capsule: Path, target: Path, label: str,
             succeeds: bool) -> dict:
    before = snapshots(target)
    # Discovery must identify the third-party function without rewriting it.
    report = json.loads(cli("suggest-rust-api", str(capsule), str(target)).stdout)
    choices = [x for x in report["requirements"] if x["target_function"] == "valid_checksum"]
    assert len(choices) == 1 and choices[0]["best_suggestion"], report
    proposed = choices[0]["best_suggestion"]["suggested_cli_rule"]
    assert proposed == RULE, proposed
    assert snapshots(target) == before, "Read-only discovery modified target"

    output = root / f"output-{label}"
    started = perf_counter()
    result = cli(
        "verify-rust", str(capsule), str(target), "--output", str(output),
        "--license-reviewed", "--registry-cache", str(CACHE),
        "--reviewed-api-rule", proposed, "--contract-test", TEST_PATH,
        require_success=False,
    )
    elapsed = round(perf_counter() - started, 2)
    assert snapshots(target) == before, f"{label} modified original target"

    if result.returncode:
        assert not output.exists(), f"{label} published after failure"
        assert not succeeds, f"{label} unexpectedly rejected: {result.stderr[-1700:]}"
        text = result.stdout + result.stderr
        assert "test result: FAILED" in text, (
            f"{label} rejected before behavioral verification: {text[-2000:]}"
        )
        print(f"PASS rejected false completion {label}: actual Cargo test failure",
              flush=True)
        return {"case": label, "status": "behavioral_failure_rejected",
                "elapsed_seconds": elapsed}

    assert succeeds, f"Wrong behavior unexpectedly published in {label}"
    evidence = json.loads((output / "composer-build-evidence.json").read_text())
    provenance = json.loads(
        (output / "composer-source-attributions.json").read_text()
    )
    assert evidence["tests_passed"] is True
    assert evidence["network_disabled"] is True
    assert evidence["reviewed_api_rule"] == proposed
    assert evidence["reviewed_contract_test"] == TEST_PATH
    assert "test result: ok" in evidence["log_tail"]
    assert provenance and provenance[0]["source_commit_sha"] == UPSTREAM_SHA
    assert provenance[0]["license_documents"]
    assert (output / "LICENSE").is_file(), "Target license lost"
    assert (output / "third_party_licenses").is_dir()
    assert "todo!()" not in (output / "src/lib.rs").read_text().split(
        "pub fn valid_checksum", 1
    )[1].split("}", 1)[0]
    print(f"PASS external Luhn capability {label}: Docker contract + full suite",
          flush=True)
    return {"case": label, "status": "verified", "elapsed_seconds": elapsed}


def main() -> None:
    run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-external-luhn-") as tmp:
        root = Path(tmp)
        upstream = root / "external-upstream"
        target = root / "public-target"
        clone_exact(UPSTREAM, UPSTREAM_SHA, upstream)
        clone_exact(TARGET_REPO, TARGET_SHA, target)
        assert (upstream / "LICENSE").is_file(), "Missing upstream license"
        assert (target / "LICENSE").is_file(), "Missing target license"
        source_sha = (upstream / "src/lib.rs").read_bytes()

        capsule = make_capsule(
            upstream, root, "valid", "src/lib.rs", "external-luhn-validation",
            {"src/lib.rs"}, expected_sha=UPSTREAM_SHA, license_file="LICENSE",
        )
        assert prepare_registry(target, root, "external-luhn-trigram"), (
            "Target registry cache preparation failed"
        )

        results = []
        for iteration in range(1, 4):
            work = target_copy(root, target, f"input-{iteration}", GOOD_CONTRACT)
            results.append(run_case(root, capsule, work, f"attempt-{iteration}",
                                    succeeds=True))
        wrong = target_copy(root, target, "input-wrong-semantic", WRONG_CONTRACT)
        results.append(run_case(root, capsule, wrong, "wrong-semantic",
                                succeeds=False))
        assert (upstream / "src/lib.rs").read_bytes() == source_sha, (
            "Upstream algorithm changed"
        )
        assert sum(x["status"] == "verified" for x in results) == 3
        assert sum(x["status"] == "behavioral_failure_rejected"
                   for x in results) == 1
        print("EVIDENCE_JSON " + json.dumps({
            "schema_version": 1,
            "external_source": UPSTREAM,
            "source_commit": UPSTREAM_SHA,
            "public_target": TARGET_REPO,
            "target_commit": TARGET_SHA,
            "repeats_verified": 3,
            "semantic_rejections_verified": 1,
            "algorithm_reimplemented": False,
            "outcomes": results,
            "limitations": (
                "Human-selected external source; hand-written independent "
                "tests and stub only. Source-to-target reuse is bounded to "
                "a reviewed scalar Rust API; not autonomous cross-repository "
                "selection, general semantic equivalence or VM isolation."
            ),
        }, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
