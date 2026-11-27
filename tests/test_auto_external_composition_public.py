"""End-to-end approved external source selection and independent verification.

No domain feature is regenerated. The only sources of algorithm implementations
are published crates from independent, SHA-pinned public projects. Target tests
are written independently; all execution uses Composer's existing offline
non-root Docker verifier.
"""
from __future__ import annotations

import json
from pathlib import Path
import shutil
import tempfile

import test_external_crate_adoption_public as upstream

CATALOG = Path(__file__).resolve().parents[1] / "examples/approved-external-capabilities.json"
EXCLUDE_USER = "KUNDURUHIMANEESHREDDY"
LUHN_SOURCE = "https://github.com/jeffcarp/luhn-rs.git"
LUHN_SHA = "dbe3684f2f4b36bccfb3699f26fab9eaf17335fc"
LUHN_TEST = """#[test]
fn external_luhn_validation_has_expected_behavior() {
    assert!(valid_checksum("4111111111111111"));
    assert!(valid_checksum("4111-1111-1111-1111"));
    assert!(!valid_checksum("4111111111111112"));
    assert!(!valid_checksum("banana"));
    assert!(!valid_checksum(""));
}
"""


def auto(target: Path, output: Path, capability: str, alias: str = "ported_jaro",
         reviewed: bool = True, catalog: Path = CATALOG, excludes: tuple[str, ...] = (EXCLUDE_USER,),
         require_success: bool = True):
    args = [
        "compose-external-rust", capability, str(target),
        "--catalog", str(catalog), "--output", str(output),
        "--target-alias", alias,
        "--contract-test", upstream.TEST_NAME,
        "--registry-cache", str(upstream.CACHE),
    ]
    for owner in excludes:
        args.extend(["--exclude-owner", owner])
    if reviewed:
        args.append("--license-reviewed")
    return upstream.cli(*args, require_success=require_success)


def independent_luhn_target(target: Path, root: Path):
    work = root / "luhn-target"
    shutil.copytree(target, work, ignore=shutil.ignore_patterns(".git"))
    tests = work / "tests"
    tests.mkdir(exist_ok=True)
    (work / upstream.TEST_NAME).write_text(
        "use trigram::valid_checksum;\n" + LUHN_TEST, encoding="utf-8"
    )
    baseline = (
        (work / "Cargo.toml").read_bytes(),
        (work / "src/lib.rs").read_bytes(),
        (work / upstream.TEST_NAME).read_bytes(),
    )
    return work, baseline


def prefetch_luhn(target: Path, root: Path):
    work = root / "prefetch-luhn"
    shutil.copytree(target, work, ignore=shutil.ignore_patterns(".git"))
    manifest = work / "Cargo.toml"
    before = manifest.read_text()
    assert "[dependencies]" in before
    manifest.write_text(before.replace(
        "[dependencies]", '[dependencies]\nluhn = "=1.0.2"', 1
    ))
    upstream.run(
        "cargo", "fetch", "--manifest-path", str(manifest),
        "--target", "x86_64-unknown-linux-gnu", timeout=180
    )


def main():
    upstream.run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-automatic-external-") as tmp:
        root = Path(tmp)
        third_party = root / "pinned-strsim"
        target = root / "pinned-trigram"
        upstream.clone_exact(upstream.SOURCE_REPO, upstream.SOURCE_SHA, third_party)
        upstream.clone_exact(upstream.TARGET_REPO, upstream.TARGET_SHA, target)
        assert (third_party / "LICENSE").is_file()
        assert (target / "LICENSE").is_file()
        upstream.prepare_registry_cache(target, root)
        results = []
        for n in range(1, 4):
            work, baseline = upstream.test_fixture(
                target, root, f"auto-jaro-{n}", upstream.GOOD_CONTRACT
            )
            output = root / f"published-auto-jaro-{n}"
            result = json.loads(auto(
                work, output, "jaro string similarity"
            ).stdout)
            choice = result["selection"]
            evidence = result["integration"]
            assert choice["package"] == "strsim"
            assert choice["approved_pinned_version"] == "0.11.1"
            assert choice["repository"] == upstream.SOURCE_REPO[:-4]
            assert evidence["offline_tests_passed"]
            assert evidence["original_target_unchanged"]
            assert evidence["locked_version"] == "0.11.1"
            assert len(evidence["registry_checksum"]) == 64
            assert evidence["provenance_commit_matched_to_registry_artifact"] is False
            assert (output / "composer-upstream-crate-evidence.json").is_file()
            assert 'pub use strsim::jaro as ported_jaro;' in (
                output / "src/lib.rs"
            ).read_text()
            upstream.original_unchanged(work, baseline)
            results.append({"run": n, "source": "strsim", "verified": True})
            print(f"PASS selected and verified independent strsim {n}/3", flush=True)

        wrong, baseline = upstream.test_fixture(
            target, root, "auto-wrong-jaro", upstream.MISLEADING_CONTRACT
        )
        wrong_output = root / "must-not-publish-wrong-answer"
        rejected = auto(
            wrong, wrong_output, "jaro string similarity", require_success=False
        )
        assert rejected.returncode != 0
        assert "test result: FAILED" in rejected.stderr, rejected.stderr[-1500:]
        assert not wrong_output.exists()
        upstream.original_unchanged(wrong, baseline)
        print("PASS wrong-but-compiling selected API rejected by independent contract", flush=True)

        unknown, baseline = upstream.test_fixture(
            target, root, "auto-unknown", upstream.GOOD_CONTRACT
        )
        unknown_output = root / "must-not-publish-unknown"
        rejected = auto(unknown, unknown_output, "unknown quantum database",
                        require_success=False)
        assert rejected.returncode != 0
        assert "no unique reviewed source" in rejected.stderr
        assert not unknown_output.exists()
        upstream.original_unchanged(unknown, baseline)

        excluded_output = root / "must-not-publish-own-source"
        rejected = auto(unknown, excluded_output, "jaro", excludes=("rapidfuzz",),
                        require_success=False)
        assert rejected.returncode != 0
        assert "excluded repository owner" in rejected.stderr
        assert not excluded_output.exists()
        upstream.original_unchanged(unknown, baseline)
        print("PASS unknown or excluded external source fails closed", flush=True)

        conflict, baseline = upstream.test_fixture(
            target, root, "auto-cargo-conflict", upstream.GOOD_CONTRACT
        )
        manifest = conflict / "Cargo.toml"
        manifest.write_text(manifest.read_text().replace(
            "[dependencies]", '[dependencies]\nstrsim = "=0.10.0"', 1
        ))
        before_conflict = (
            manifest.read_bytes(), baseline[1], baseline[2]
        )
        conflict_output = root / "must-not-publish-conflict"
        rejected = auto(
            conflict, conflict_output, "jaro", require_success=False
        )
        assert rejected.returncode != 0
        assert "already declares strsim" in rejected.stderr
        assert not conflict_output.exists()
        upstream.original_unchanged(conflict, before_conflict)
        print("PASS existing Cargo version conflict refused", flush=True)

        unreviewed_output = root / "must-not-publish-unreviewed"
        rejected = auto(unknown, unreviewed_output, "jaro", reviewed=False,
                        require_success=False)
        assert rejected.returncode != 0
        assert "review required" in rejected.stderr or "review required" in (
            rejected.stdout + rejected.stderr
        ).lower() or "review" in rejected.stderr.lower()
        assert not unreviewed_output.exists()
        print("PASS source/license review gate", flush=True)

        luhn = root / "pinned-luhn"
        upstream.clone_exact(LUHN_SOURCE, LUHN_SHA, luhn)
        assert (luhn / "LICENSE").is_file()
        prefetch_luhn(target, root)
        luhn_target, luhn_baseline = independent_luhn_target(target, root)
        luhn_output = root / "published-external-luhn"
        luhn_result = json.loads(auto(
            luhn_target, luhn_output, "luhn validation",
            alias="valid_checksum"
        ).stdout)
        assert luhn_result["selection"]["package"] == "luhn"
        assert luhn_result["integration"]["locked_version"] == "1.0.2"
        assert luhn_result["integration"]["offline_tests_passed"] is True
        assert 'pub use luhn::valid as valid_checksum;' in (
            luhn_output / "src/lib.rs"
        ).read_text()
        upstream.original_unchanged(luhn_target, luhn_baseline)
        print("PASS second independent upstream luhn capability", flush=True)
        results.append({"run": 4, "source": "luhn", "verified": True})

        print("EVIDENCE_JSON " + json.dumps({
            "auto_reviewed_capability_selection": True,
            "external_sources": [upstream.SOURCE_REPO, LUHN_SOURCE],
            "independent_public_target": upstream.TARGET_REPO,
            "verified_runs": results,
            "invalid_behavior_rejected_by_cargo": True,
            "unknown_or_excluded_source_rejected": True,
            "dependency_conflict_rejected": True,
            "unreviewed_source_rejected": True,
            "remaining": (
                "Approved catalog only; manual API contracts and source/license "
                "review still required. SHA not cryptographically matched to "
                "registry release. Not arbitrary autonomous GitHub discovery."
            ),
        }, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
