"""Live recovery: wrong behavior -> independent upstream alternative -> verified.

Imports ONLY existing third-party crates, never generates algorithm source.
The user-supplied candidate set is reviewed; Composer selects the first crate
that passes independently authored target-owned behavioral tests.
"""
from __future__ import annotations

import json
import shutil
import tempfile
from pathlib import Path

from test_pinned_public_multifile import CACHE, clone_exact, run

UPSTREAM_ONE = ("https://github.com/rapidfuzz/strsim-rs.git",
                "dacc84c0dc61eff0ee0ff66962bcf2e17018ad26")
UPSTREAM_TWO = ("https://github.com/life4/textdistance.rs.git",
                "0f92088dfc84a99a7652b3f6d7e268afd98dbaa6")
TARGET = ("https://github.com/ijt/trigram.git",
          "45af254dcd7b5c8aac4feb336192204791cd1e2f")
CONTRACT = "tests/composer_external_recovery_contract.rs"

JARO_CONTRACT = """use trigram::ported_jaro;
#[test]
fn independent_jaro_behavior() {
    assert!((ported_jaro("", "") - 1.0).abs() < 1e-12);
    assert!((ported_jaro("same", "same") - 1.0).abs() < 1e-12);
    assert!(ported_jaro("abc", "xyz").abs() < 1e-12);
    assert!((ported_jaro("MARTHA", "MARHTA") - 17.0 / 18.0).abs() < 1e-12);
}
"""


def candidate(repo: tuple[str, str], package: str, symbol: str,
              version_requirement: str) -> dict:
    return {
        "package": package,
        "version_requirement": version_requirement,
        "public_symbol": symbol,
        "upstream_repository": repo[0][:-4],
        "upstream_commit": repo[1],
    }


BAD = candidate(UPSTREAM_ONE, "strsim", "jaro_winkler", "0.11.1")
GOOD = candidate(UPSTREAM_TWO, "textdistance", "str::jaro", "^1.1")
STRSIM_GOOD = candidate(UPSTREAM_ONE, "strsim", "jaro", "^0.11")


def cli(*args: str):
    return run("cargo", "run", "--quiet", "-p", "composer-cli", "--", *args, timeout=300)


def create_target(source: Path, root: Path, name: str, conflict: bool = False):
    target = root / f"target-{name}"
    shutil.copytree(source, target, ignore=shutil.ignore_patterns(".git"))
    (target / "tests").mkdir(exist_ok=True)
    (target / CONTRACT).write_text(JARO_CONTRACT, encoding="utf-8")
    if conflict:
        manifest = target / "Cargo.toml"
        body = manifest.read_text()
        assert "[dependencies]" in body
        manifest.write_text(body.replace(
            "[dependencies]", '[dependencies]\nstrsim = "=0.10.0"', 1
        ))
    snapshots = (
        (target / "Cargo.toml").read_bytes(),
        (target / "src/lib.rs").read_bytes(),
        (target / CONTRACT).read_bytes(),
    )
    return target, snapshots


def preserved(target: Path, before) -> None:
    assert (target / "Cargo.toml").read_bytes() == before[0]
    assert (target / "src/lib.rs").read_bytes() == before[1]
    assert (target / CONTRACT).read_bytes() == before[2]


def fetch_real_crates(target: Path, root: Path) -> None:
    """Pre-cache released archives using Cargo fetch only, without building source."""
    src = root / "prefetch"
    shutil.copytree(target, src, ignore=shutil.ignore_patterns(".git"))
    toml = src / "Cargo.toml"
    content = toml.read_text()
    toml.write_text(content.replace(
        "[dependencies]",
        "[dependencies]\n"
        'composer_strsim_old = { package = "strsim", version = "=0.10.0" }\n'
        'composer_strsim_new = { package = "strsim", version = "=0.11.1" }\n'
        'textdistance = "=1.1.1"',
        1,
    ))
    run("cargo", "fetch", "--manifest-path", str(toml),
        "--target", "x86_64-unknown-linux-gnu", timeout=180)
    for prefix in ["strsim-0.10.0.crate", "strsim-0.11.1.crate",
                   "textdistance-1.1.1.crate"]:
        matches = list((CACHE / "cache").glob(f"index.crates.io-*/{prefix}"))
        assert matches, f"required upstream registry artifact was not cached: {prefix}"


def recover(target: Path, root: Path, name: str, candidates: list[dict]):
    manifest = root / f"{name}-candidates.json"
    manifest.write_text(json.dumps(candidates), encoding="utf-8")
    output = root / f"verified-{name}"
    report = json.loads(cli(
        "recover-external-rust-crate", str(target),
        "--output", str(output),
        "--candidates-json", str(manifest),
        "--target-alias", "ported_jaro",
        "--contract-test", CONTRACT,
        "--registry-cache", str(CACHE),
        "--license-reviewed",
    ).stdout)
    return report, output


def fallback_to_another_public_repo(source: Path, root: Path) -> None:
    target, before = create_target(source, root, "multi-source")
    report, output = recover(target, root, "multi-source", [BAD, GOOD])
    assert report["completed"] is True, report
    assert report["selected_package"] == "textdistance", report
    assert report["selected_symbol"] == "str::jaro", report
    assert report["selected_version"] == "1.1.1", report
    assert len(report["attempts"]) == 2, report
    assert report["attempts"][0]["status"] == "rejected", report
    assert report["attempts"][1]["status"] == "verified", report
    assert report["attempts"][0]["package"] == "strsim"
    assert report["attempts"][1]["package"] == "textdistance"
    assert output.is_dir()
    manifest = (output / "Cargo.toml").read_text()
    assert '=1.1.1' in manifest, manifest
    assert "pub use textdistance::str::jaro as ported_jaro" in (output / "src/lib.rs").read_text()
    proof = json.loads((output / "composer-recovery-evidence.json").read_text())
    assert proof["network_disabled"] is True
    assert proof["tests_passed"] is True
    assert proof["provenance_commit_matched_to_registry_artifact"] is False
    preserved(target, before)
    print("PASS first upstream failed behavioral tests; second independent repo passed", flush=True)


def recover_version_conflict(source: Path, root: Path) -> None:
    target, before = create_target(source, root, "cargo-isolation", conflict=True)
    report, output = recover(target, root, "cargo-isolation", [STRSIM_GOOD])
    assert report["completed"] is True, report
    assert report["selected_package"] == "strsim"
    assert report["selected_version"] == "0.11.1"
    assert report["attempts"][0]["isolated_dependency"] is True
    manifest = (output / "Cargo.toml").read_text()
    assert '=0.10.0' in manifest and '=0.11.1' in manifest, manifest
    assert "composer_strsim" in manifest
    assert "pub use composer_strsim::jaro as ported_jaro" in (output / "src/lib.rs").read_text()
    assert (output / "composer-recovery-evidence.json").is_file()
    preserved(target, before)
    print("PASS conflicting existing strsim 0.10.0 preserved; strsim 0.11.1 isolated and verified", flush=True)


def resolve_unpinned_version(source: Path, root: Path) -> None:
    target, before = create_target(source, root, "cached-latest")
    broad = candidate(UPSTREAM_TWO, "textdistance", "str::jaro", "latest")
    report, output = recover(target, root, "cached-latest", [broad])
    assert report["completed"] is True, report
    assert report["selected_version"] == "1.1.1", report
    assert '=1.1.1' in (output / "Cargo.toml").read_text()
    assert report["tests_passed"] is True
    preserved(target, before)
    print("PASS broad latest cached request resolved to exact verified release", flush=True)


def all_candidates_fail_without_publication(source: Path, root: Path) -> None:
    target, before = create_target(source, root, "all-failed")
    report, output = recover(target, root, "all-failed", [BAD])
    assert report["completed"] is False, report
    assert report["tests_passed"] is False
    assert report["output_root"] is None
    assert report["attempts"][0]["status"] == "rejected"
    assert report["evidence_path"] is None
    assert not output.exists()
    preserved(target, before)
    print("PASS exhaustive failure reported, no output published and original untouched", flush=True)


def main() -> None:
    run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-multi-upstream-recovery-") as temp:
        root = Path(temp)
        src_a = root / "real-strsim"
        src_b = root / "real-textdistance"
        target = root / "real-trigram"
        clone_exact(*UPSTREAM_ONE, src_a)
        clone_exact(*UPSTREAM_TWO, src_b)
        clone_exact(*TARGET, target)
        import tomllib
        for path, expected in [(src_a, "strsim"), (src_b, "textdistance")]:
            manifest = tomllib.loads((path / "Cargo.toml").read_text())
            assert manifest["package"]["name"] == expected
            assert "MIT" in manifest["package"]["license"]
            assert (path / "LICENSE").is_file()
        assert (target / "LICENSE").is_file()
        fetch_real_crates(target, root)
        fallback_to_another_public_repo(target, root)
        recover_version_conflict(target, root)
        resolve_unpinned_version(target, root)
        all_candidates_fail_without_publication(target, root)
        print("EVIDENCE_JSON " + json.dumps({
            "different_source_recovery": "verified",
            "conflicting_versions_coexist": "verified",
            "broad_version_locked": "verified",
            "all_failure_no_publish": "verified",
            "source_a": UPSTREAM_ONE,
            "source_b": UPSTREAM_TWO,
            "target": TARGET,
            "limitations": "Reviewed candidate list supplied by tester, not autonomous web search. "
                           "Version choice limited to pre-cached stable crates.io releases. "
                           "Behavioral assertions are independent but incomplete. "
                           "GitHub commit-to-registry correspondence is not established.",
        }, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
