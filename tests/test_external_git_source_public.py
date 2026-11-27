"""Live external Git source transfer, using Cargo's own pinned Git resolver.

Git fetch is performed in a disposable prefetch project; no source code is
built or executed on the host. Actual integration and behavioral tests happen
only in Composer's existing network-disabled Docker verifier.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import tempfile

from test_pinned_public_multifile import CACHE, clone_exact, run

SOURCE_REPO = "https://github.com/rapidfuzz/strsim-rs.git"
SOURCE_SHA = "dacc84c0dc61eff0ee0ff66962bcf2e17018ad26"
TARGET_REPO = "https://github.com/ijt/trigram.git"
TARGET_SHA = "45af254dcd7b5c8aac4feb336192204791cd1e2f"
CONTRACT = "tests/composer_git_upstream_contract.rs"
GIT_CACHE = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))) / "git"

CONTRACT_TEST = """use trigram::ported_jaro;
#[test]
fn independently_verify_git_jaro() {
    assert!((ported_jaro("", "") - 1.0).abs() < 1e-12);
    assert!((ported_jaro("same", "same") - 1.0).abs() < 1e-12);
    assert!(ported_jaro("abc", "xyz").abs() < 1e-12);
    assert!((ported_jaro("MARTHA", "MARHTA") - 17.0 / 18.0).abs() < 1e-12);
}
"""


def cli(*args: str, require_success: bool = True):
    return run("cargo", "run", "--quiet", "-p", "composer-cli", "--",
               *args, require_success=require_success, timeout=300)


def make_target(original: Path, root: Path, key: str, conflict: bool = False):
    target = root / f"target-{key}"
    shutil.copytree(original, target, ignore=shutil.ignore_patterns(".git"))
    (target / "tests").mkdir(exist_ok=True)
    (target / CONTRACT).write_text(CONTRACT_TEST, encoding="utf-8")
    if conflict:
        file = target / "Cargo.toml"
        text = file.read_text()
        assert "[dependencies]" in text
        file.write_text(text.replace("[dependencies]",
                                     '[dependencies]\nstrsim = "=0.10.0"', 1))
    snapshots = [(target / path).read_bytes() for path in
                 ["Cargo.toml", "src/lib.rs", CONTRACT]]
    return target, snapshots


def assert_unchanged(target: Path, snapshots) -> None:
    for name, before in zip(["Cargo.toml", "src/lib.rs", CONTRACT], snapshots):
        assert (target / name).read_bytes() == before


def prefetch_git(target: Path, root: Path) -> None:
    """Provision Cargo Git DB/checkouts and crates.io registry; no host build."""
    project = root / "prefetch-git"
    shutil.copytree(target, project, ignore=shutil.ignore_patterns(".git"))
    manifest = project / "Cargo.toml"
    text = manifest.read_text()
    assert "[dependencies]" in text
    manifest.write_text(text.replace("[dependencies]",
        '[dependencies]\n'
        f'composer_git_strsim = {{ package = "strsim", git = "{SOURCE_REPO}", rev = "{SOURCE_SHA}" }}\n'
        'strsim_legacy = { package = "strsim", version = "=0.10.0" }', 1))
    run("cargo", "fetch", "--manifest-path", str(manifest),
        "--target", "x86_64-unknown-linux-gnu", timeout=180)
    assert GIT_CACHE.is_dir(), "cargo fetch did not populate git cache"
    assert (GIT_CACHE / "checkouts").is_dir(), "git checkout cache absent"
    assert (GIT_CACHE / "db").is_dir(), "git database cache absent"
    assert CACHE.is_dir(), "registry cache absent"


def execute(target: Path, root: Path, source: Path, name: str,
            symbol: str = "jaro", sha: str = SOURCE_SHA,
            license_flag: bool = True, require_success: bool = True):
    out = root / f"verified-{name}"
    args = [
        "verify-external-git-rust", str(target),
        "--output", str(out),
        "--upstream-checkout", str(source),
        "--upstream-repository", SOURCE_REPO[:-4],
        "--upstream-commit", sha,
        "--package", "strsim",
        "--public-symbol", symbol,
        "--target-alias", "ported_jaro",
        "--contract-test", CONTRACT,
        "--registry-cache", str(CACHE),
        "--git-cache", str(GIT_CACHE),
    ]
    if license_flag:
        args.append("--license-reviewed")
    return cli(*args, require_success=require_success), out


def positive_github_reuse(target_source: Path, upstream: Path, root: Path) -> None:
    for repeat, conflict in [(1, False), (2, True)]:
        target, snapshots = make_target(target_source, root, str(repeat), conflict)
        result, out = execute(target, root, upstream, f"success-{repeat}")
        data = json.loads(result.stdout)
        assert data["offline_tests_passed"] is True
        assert data["lockfile_git_commit_verified"] is True
        assert data["source_checkout_commit_verified"] is True
        assert data["upstream_commit"] == SOURCE_SHA
        assert len(data["source_license_sha256"]) == 64
        assert (out / "third_party_licenses/strsim-git-license.txt").is_file()
        src = (out / "src/lib.rs").read_text()
        assert "pub use composer_git_strsim::jaro as ported_jaro" in src
        assert SOURCE_SHA in (out / "Cargo.toml").read_text()
        lock = (out / "Cargo.lock").read_text()
        assert SOURCE_SHA in lock and "git+https://github.com/rapidfuzz/strsim-rs" in lock
        if conflict:
            assert '=0.10.0' in (out / "Cargo.toml").read_text()
        evidence = json.loads((out / "composer-upstream-git-evidence.json").read_text())
        assert evidence["cargo_lock_commit_verified"] is True
        assert evidence["source_checkout_commit_verified"] is True
        assert evidence["network_disabled"] is True
        assert evidence["offline_tests_passed"] is True
        assert "test result: ok" in evidence["log_tail"]
        assert_unchanged(target, snapshots)
        print(f"PASS exact-Git-source capability reused repeat={repeat} "
              f"conflicting-dependency-isolated={conflict} Docker={data['elapsed_seconds']:.2f}s",
              flush=True)


def negatives(target_source: Path, upstream: Path, root: Path) -> None:
    target, snapshot = make_target(target_source, root, "negative-git")
    response, out = execute(target, root, upstream, "wrong-behavior",
                            symbol="jaro_winkler", require_success=False)
    assert response.returncode != 0
    assert "test result: FAILED" in response.stderr, response.stderr[-3000:]
    assert not out.exists()
    assert_unchanged(target, snapshot)
    print("PASS behaviorally incorrect, type-compatible Git implementation rejected", flush=True)

    response, out = execute(target, root, upstream, "wrong-commit",
                            sha="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                            require_success=False)
    assert response.returncode != 0
    assert "checkout HEAD differs" in response.stderr, response.stderr[-1500:]
    assert not out.exists()
    assert_unchanged(target, snapshot)
    print("PASS requested SHA mismatching reviewed checkout rejected", flush=True)

    response, out = execute(target, root, upstream, "no-review",
                            license_flag=False, require_success=False)
    assert response.returncode != 0
    assert "source/license review" in response.stderr, response.stderr[-1500:]
    assert not out.exists()
    assert_unchanged(target, snapshot)
    print("PASS Git source without explicit license review rejected", flush=True)


def main() -> None:
    run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-public-git-reuse-") as temp:
        root = Path(temp)
        upstream = root / "real-strsim-git"
        target = root / "public-trigram"
        clone_exact(SOURCE_REPO, SOURCE_SHA, upstream)
        clone_exact(TARGET_REPO, TARGET_SHA, target)
        assert (upstream / "LICENSE").is_file()
        assert (target / "LICENSE").is_file()
        prefetch_git(target, root)
        positive_github_reuse(target, upstream, root)
        negatives(target, upstream, root)
        print("EVIDENCE_JSON " + json.dumps({
            "source_repository": SOURCE_REPO,
            "source_git_sha": SOURCE_SHA,
            "target_repository": TARGET_REPO,
            "target_git_sha": TARGET_SHA,
            "git_reuse_verified_runs": 2,
            "git_lockfile_sha_verified": True,
            "source_checkout_verified": True,
            "license_evidence_preserved": True,
            "incorrect_behavior_refused": True,
            "source_sha_mismatch_refused": True,
            "unreviewed_source_refused": True,
            "originals_unchanged": True,
            "limitations": "Only Rust git dependencies from independently reviewed public GitHub "
                          "libraries; external crates fetched via Cargo, built only in restricted "
                          "offline Docker. Does not prove general Git-only repository suitability "
                          "or complete semantic equivalence, and Docker is not VM-grade isolation.",
        }, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
