"""Held-out Rust API transfer matrix with explicit supported/unsupported outcomes.

A previously verified public source is tested against an unfamiliar real target.
A separate unfamiliar public source deliberately exercises the unresolved-import
fail-closed path. Wrong-but-compilable semantics and disjoint dependency ranges
must also be rejected. No arbitrary-source reliability inference is permitted.
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

# Unseen relative to the earlier TheAlgorithms/EbTech/rustc-hash matrix.
SOURCE_REPO = "https://github.com/TheAlgorithms/Rust.git"
SOURCE_SHA = "976f1cdcdab3fab85301c38b28c28e6c5c503e9a"
UNSUPPORTED_REPO = "https://github.com/rapidfuzz/strsim-rs.git"
UNSUPPORTED_SHA = "dacc84c0dc61eff0ee0ff66962bcf2e17018ad26"
TARGET_REPO = "https://github.com/ijt/trigram.git"
TARGET_SHA = "45af254dcd7b5c8aac4feb336192204791cd1e2f"
RULE = "euler_totient=adapted_totient(u32)->u128"
TEST = "tests/composer_heldout_contract.rs"
REPEATS = 3


def add_contract(root: Path, test_body: str) -> tuple[bytes, bytes, bytes]:
    lib = root / "src/lib.rs"
    manifest = root / "Cargo.toml"
    original = lib.read_bytes()
    lib.write_bytes(original + b"\npub fn adapted_totient(n: u32) -> u128 { todo!() }\n")
    (root / "tests").mkdir(exist_ok=True)
    test = root / TEST
    test.write_text("use trigram::adapted_totient;\n" + test_body, encoding="utf-8")
    return lib.read_bytes(), manifest.read_bytes(), test.read_bytes()


GOOD_CONTRACT = """#[test]
fn independent_totient_edges() {
    assert_eq!(adapted_totient(1_u32), 1_u128);
    assert_eq!(adapted_totient(9_u32), 6_u128);
    assert_eq!(adapted_totient(10_u32), 4_u128);
    assert_eq!(adapted_totient(13_u32), 12_u128);
    assert_eq!(adapted_totient(36_u32), 12_u128);
}
"""

# Both the source and the desired implementation compile with the same API,
# but "number of divisors" (3 for 9) is NOT Euler's totient (6 for 9).
MISLEADING_CONTRACT = """#[test]
fn incorrect_but_type_correct_divisor_semantics() {
    assert_eq!(adapted_totient(9_u32), 3_u128);
}
"""

def verify_positive(source: Path, target: Path, capsule: Path, root: Path) -> list[dict]:
    results: list[dict] = []
    for iteration in range(1, REPEATS + 1):
        working = root / f"totient-target-{iteration}"
        shutil.copytree(target, working, ignore=shutil.ignore_patterns(".git"))
        before = (target / "src/lib.rs").read_bytes()
        snapshots = add_contract(working, GOOD_CONTRACT)
        observed = json.loads(cli("suggest-rust-api", str(capsule), str(working)).stdout)
        requirements = [x for x in observed["requirements"] if x["target_function"] == "adapted_totient"]
        assert len(requirements) == 1 and requirements[0]["best_suggestion"], observed
        rule = requirements[0]["best_suggestion"]["suggested_cli_rule"]
        assert rule == RULE, (rule, RULE)
        output = root / f"totient-verified-{iteration}"
        started = perf_counter()
        verified = json.loads(cli(
            "verify-rust", str(capsule), str(working),
            "--output", str(output), "--license-reviewed",
            "--registry-cache", str(CACHE),
            "--reviewed-api-rule", rule, "--contract-test", TEST,
        ).stdout)
        seconds = round(perf_counter() - started, 2)
        assert verified["isolated_tests_passed"] is True
        assert verified["reviewed_api_rule"] == RULE
        assert verified["reviewed_contract_test"] == TEST
        evidence = json.loads((output / "composer-build-evidence.json").read_text())
        assert evidence["tests_passed"] is True
        assert evidence["network_disabled"] is True
        assert "test result: ok" in evidence["log_tail"]
        assert "pub fn adapted_totient" in (output / "src/lib.rs").read_text()
        assert (working / "src/lib.rs").read_bytes() == snapshots[0]
        assert (working / "Cargo.toml").read_bytes() == snapshots[1]
        assert (working / TEST).read_bytes() == snapshots[2]
        assert (target / "src/lib.rs").read_bytes() == before
        print(f"PASS heldout_target_totient repeat={iteration}/{REPEATS} docker_seconds={seconds}", flush=True)
        results.append({"run": iteration, "status": "verified",
                        "docker_seconds": seconds, "original_unchanged": True})
    return results


def require_negative_failure(response, destination: Path, original: tuple[bytes, bytes, bytes],
                             target: Path, label: str, error_marker: str | None = None):
    assert response.returncode != 0, f"{label} unexpectedly passed"
    assert not destination.exists(), f"{label}: failed integration published output"
    assert (target / "src/lib.rs").read_bytes() == original[0]
    assert (target / "Cargo.toml").read_bytes() == original[1]
    assert (target / TEST).read_bytes() == original[2]
    stderr = response.stderr + response.stdout
    if error_marker is not None:
        assert error_marker in stderr, (label, stderr[-2500:])
    print(f"PASS rejected {label}: fail-closed and original files unchanged", flush=True)


def verify_misleading_semantics(target: Path, capsule: Path, root: Path) -> None:
    work = root / "misleading-totient-target"
    shutil.copytree(target, work, ignore=shutil.ignore_patterns(".git"))
    snapshots = add_contract(work, MISLEADING_CONTRACT)
    report = json.loads(cli("suggest-rust-api", str(capsule), str(work)).stdout)
    req = next(x for x in report["requirements"] if x["target_function"] == "adapted_totient")
    assert req["best_suggestion"]["suggested_cli_rule"] == RULE
    destination = root / "never-publish-wrong-semantic-match"
    response = cli(
        "verify-rust", str(capsule), str(work), "--output", str(destination),
        "--license-reviewed", "--registry-cache", str(CACHE),
        "--reviewed-api-rule", RULE, "--contract-test", TEST,
        require_success=False,
    )
    # The wrong implementation must reach Cargo tests and fail assertions;
    # pre-admission errors and compilation failures do not satisfy the probe.
    require_negative_failure(response, destination, snapshots, work,
                             "wrong-but-compilable totient semantics", "test result: FAILED")


def verify_dependency_conflict(target: Path, root: Path) -> None:
    """Synthetic source + untouched public target copy with disjoint semver ranges.

    The source uses semver 1.x; a disposable target copy declares 0.11.x.
    Acceptance requires rejection and no publication. This case is a controlled
    dependency fixture, not a third independent public-source integration.
    """
    source = root / "dependency-source-fixture"
    (source / "src").mkdir(parents=True)
    (source / "Cargo.toml").write_text(
        "[package]\nname = 'heldout_semver_source'\nversion = '0.1.0'\n"
        "edition = '2021'\nlicense = 'MIT'\n[dependencies]\nsemver = '1'\n",
        encoding="utf-8",
    )
    (source / "src/lib.rs").write_text(
        "use semver::Version;\n"
        "pub fn parse_major(value: &str) -> u64 {\n"
        "    Version::parse(value).map(|v| v.major).unwrap_or(0)\n}\n",
        encoding="utf-8",
    )
    (source / "LICENSE").write_text(
        "MIT-licensed synthetic fixture authored solely for this benchmark.\n",
        encoding="utf-8",
    )
    run("git", "init", "-q", str(source))
    run("git", "-C", str(source), "add", ".")
    run("git", "-C", str(source), "-c", "user.name=ComposerBench",
        "-c", "user.email=benchmark@invalid.example", "commit", "-qm", "source fixture")
    sha = run("git", "-C", str(source), "rev-parse", "HEAD").stdout.strip()
    capsule = make_capsule(
        source, root, "parse_major", "src/lib.rs", "heldout-dependency-conflict",
        {"src/lib.rs"}, expected_sha=sha,
    )
    work = root / "conflict-target"
    shutil.copytree(target, work, ignore=shutil.ignore_patterns(".git"))
    lib = work / "src/lib.rs"
    cargo = work / "Cargo.toml"
    lib.write_text(lib.read_text() + "\npub fn parse_major(value: &str) -> u64 { todo!() }\n")
    text = cargo.read_text()
    assert "[dependencies]" in text
    cargo.write_text(text.replace("[dependencies]", "[dependencies]\nsemver = '=0.11.0'", 1))
    test = work / TEST
    (work / "tests").mkdir(exist_ok=True)
    test.write_text(
        "use trigram::parse_major;\n"
        "#[test] fn semver_contract() { assert_eq!(parse_major(\"1.2.3\"), 1); }\n",
        encoding="utf-8",
    )
    original = lib.read_bytes(), cargo.read_bytes(), test.read_bytes()
    assert prepare_registry(source, root, "source-semver"), "source semver cache unavailable"
    assert prepare_registry(work, root, "target-semver"), "target semver cache unavailable"
    suggestion = json.loads(cli("suggest-rust-api", str(capsule), str(work)).stdout)
    req = next(x for x in suggestion["requirements"] if x["target_function"] == "parse_major")
    rule = req["best_suggestion"]["suggested_cli_rule"]
    assert rule == "parse_major=parse_major(&str)->u64"
    output = root / "never-publish-dependency-conflict"
    response = cli(
        "verify-rust", str(capsule), str(work), "--output", str(output),
        "--license-reviewed", "--registry-cache", str(CACHE),
        "--reviewed-api-rule", rule, "--contract-test", TEST,
        require_success=False,
    )
    require_negative_failure(response, output, original, work,
                             "disjoint Cargo semver requirements",
                             "incompatible with source")



def verify_unsupported_strsim_public_source(target: Path, root: Path) -> None:
    """Unfamiliar source with unresolved references must not be executed."""
    source = root / "unfamiliar-strsim"
    clone_exact(UNSUPPORTED_REPO, UNSUPPORTED_SHA, source)
    capsule = make_capsule(
        source, root, "jaro", "src/lib.rs", "heldout-unsupported-strsim",
        {"src/lib.rs"}, expected_sha=UNSUPPORTED_SHA,
    )
    work = root / "unsupported-strsim-target"
    shutil.copytree(target, work, ignore=shutil.ignore_patterns(".git"))
    lib = work / "src/lib.rs"
    lib.write_text(lib.read_text() + "\npub fn jaro(a: &str, b: &str) -> f64 { todo!() }\n")
    (work / "tests").mkdir(exist_ok=True)
    test = work / TEST
    test.write_text(
        "use trigram::jaro;\n"
        "#[test] fn contract() { assert_eq!(jaro(\"abc\", \"abc\"), 1.0); }\n",
        encoding="utf-8",
    )
    snapshot = lib.read_bytes(), (work / "Cargo.toml").read_bytes(), test.read_bytes()
    report = json.loads(cli("suggest-rust-api", str(capsule), str(work)).stdout)
    req = next(x for x in report["requirements"] if x["target_function"] == "jaro")
    assert req["best_suggestion"] is not None
    rule = req["best_suggestion"]["suggested_cli_rule"]
    assert rule == "jaro=jaro(&str,&str)->f64"
    output = root / "must-not-publish-strsim"
    response = cli(
        "verify-rust", str(capsule), str(work), "--output", str(output),
        "--license-reviewed", "--registry-cache", str(CACHE),
        "--reviewed-api-rule", rule, "--contract-test", TEST,
        require_success=False,
    )
    require_negative_failure(response, output, snapshot, work,
                             "unresolved unfamiliar public source",
                             "unresolved imports or unfollowed symbol references")


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="composer-heldout-semantic-") as directory:
        root = Path(directory)
        source = root / "source-thealgorithms"
        target = root / "target-trigram"
        run("docker", "image", "inspect", "rust:1.99-slim")
        clone_exact(SOURCE_REPO, SOURCE_SHA, source)
        clone_exact(TARGET_REPO, TARGET_SHA, target)
        assert (source / "LICENSE").is_file() and (target / "LICENSE").is_file()
        capsule = make_capsule(
            source, root, "euler_totient", "src/number_theory/euler_totient.rs",
            "heldout-totient-to-trigram",
            {"src/number_theory/euler_totient.rs"}, expected_sha=SOURCE_SHA,
        )
        assert prepare_registry(target, root, "trigram"), "target cache unavailable"
        positive = verify_positive(source, target, capsule, root)
        verify_misleading_semantics(target, capsule, root)
        verify_unsupported_strsim_public_source(target, root)
        verify_dependency_conflict(target, root)
        evidence = {
            "schema_version": 2,
            "known_source": SOURCE_REPO, "source_sha": SOURCE_SHA,
            "heldout_target": TARGET_REPO, "target_sha": TARGET_SHA,
            "heldout_unsupported_source": UNSUPPORTED_REPO,
            "heldout_unsupported_sha": UNSUPPORTED_SHA,
            "public_target_attempts": len(positive),
            "public_target_successes": sum(x["status"] == "verified" for x in positive),
            "public_target_runs": positive,
            "wrong_but_compilable_rejected": True,
            "unresolved_unfamiliar_source_rejected": True,
            "disjoint_dependency_rejected": True,
            "limits": "Known public source to new public target; newly held-out source "
                      "correctly refused unresolved closure. Dependency mismatch is "
                      "a synthetic source fixture. Manually authored contracts; "
                      "not a general semantic reliability estimate.",
        }
        print("EVIDENCE_JSON " + json.dumps(evidence, sort_keys=True), flush=True)
        print("PASS heldout matrix: three known-source/new-public-target transfers, "
              "wrong semantics rejected, new public source unsupported "
              "and conflict fail-closed", flush=True)


if __name__ == "__main__":
    main()
