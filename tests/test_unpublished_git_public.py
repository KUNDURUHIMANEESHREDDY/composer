"""Live acceptance for an independently maintained, unpublished Git-only Rust library.

Reuse TheAlgorithms/Rust through Cargo's EXISTING pinned-Git dependency
resolver. No implementation is generated or copied from the upstream source.
Source is fetched, not compiled, on the host; all target code executes inside
Composer's existing offline, resource-constrained Docker Cargo verifier.
"""
from __future__ import annotations

import json
import os
import shutil
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

from test_pinned_public_multifile import CACHE, clone_exact, run

UPSTREAM_URL = "https://github.com/TheAlgorithms/Rust.git"
UPSTREAM_SHA = "976f1cdcdab3fab85301c38b28c28e6c5c503e9a"
PACKAGE = "the_algorithms_rust"
TARGET_URL = "https://github.com/ijt/trigram.git"
TARGET_SHA = "45af254dcd7b5c8aac4feb336192204791cd1e2f"
CONTRACT = "tests/composer_unpublished_totient_contract.rs"
GIT_CACHE = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))) / "git"

ORACLE = """use trigram::ported_totient;
#[test]
fn independent_totient_contract() {
    assert_eq!(ported_totient(1), 1);
    assert_eq!(ported_totient(2), 1);
    assert_eq!(ported_totient(9), 6);
    assert_eq!(ported_totient(10), 4);
    assert_eq!(ported_totient(13), 12);
    assert_eq!(ported_totient(36), 12);
    assert_eq!(ported_totient(100), 40);
}
"""


def cli(*args: str, require_success: bool = True, timeout: int = 320):
    return run("cargo", "run", "--quiet", "-p", "composer-cli", "--",
               *args, require_success=require_success, timeout=timeout)


def verify_unpublished_on_registry() -> None:
    """Fail when the selected Git-only package has an actual crates.io release."""
    url = "https://crates.io/api/v1/crates/" + PACKAGE
    req = urllib.request.Request(url, headers={
        "User-Agent": "Composer-public-Git-only-acceptance/1.0",
        "Accept": "application/json",
    })
    try:
        with urllib.request.urlopen(req, timeout=18) as response:
            content = response.read(1024)
            raise AssertionError(
                "Selected Git-only package exists on crates.io "
                f"(HTTP {response.status}, response {content[:120]!r})"
            )
    except urllib.error.HTTPError as error:
        assert error.code == 404, f"crates.io did not confirm absence: {error.code}"
    print("PASS upstream package absent from crates.io registry (HTTP 404)", flush=True)


def make_target(source: Path, root: Path, name: str):
    target = root / f"target-{name}"
    shutil.copytree(source, target, ignore=shutil.ignore_patterns(".git"))
    (target / "tests").mkdir(exist_ok=True)
    (target / CONTRACT).write_text(ORACLE, encoding="utf-8")
    snapshots = {
        file: (target / file).read_bytes()
        for file in ["Cargo.toml", "src/lib.rs", CONTRACT]
    }
    return target, snapshots


def unchanged(target: Path, snapshots) -> None:
    for name, baseline in snapshots.items():
        assert (target / name).read_bytes() == baseline, name


def prepare_git_cache(target: Path, root: Path) -> None:
    """Use Cargo fetch only; no build.rs or user code executes on the host."""
    prefetch = root / "prefetch"
    shutil.copytree(target, prefetch, ignore=shutil.ignore_patterns(".git"))
    manifest = prefetch / "Cargo.toml"
    content = manifest.read_text()
    assert "[dependencies]" in content
    content = content.replace(
        "[dependencies]",
        "[dependencies]\n"
        f'composer_git_the_algorithms_rust = {{ package = "{PACKAGE}", '
        f'git = "{UPSTREAM_URL}", rev = "{UPSTREAM_SHA}" }}',
        1,
    )
    manifest.write_text(content)
    run("cargo", "fetch", "--manifest-path", str(manifest),
        "--target", "x86_64-unknown-linux-gnu", timeout=220)
    assert GIT_CACHE.is_dir() and (GIT_CACHE / "checkouts").is_dir()
    assert CACHE.is_dir()


def execute(target: Path, root: Path, source: Path, name: str,
            symbol: str = "number_theory::euler_totient",
            sha: str = UPSTREAM_SHA, review: bool = True,
            require_success: bool = True):
    out = root / ("published-" + name)
    args = [
        "verify-external-git-rust", str(target),
        "--output", str(out),
        "--upstream-checkout", str(source),
        "--upstream-repository", UPSTREAM_URL[:-4],
        "--upstream-commit", sha,
        "--package", PACKAGE,
        "--public-symbol", symbol,
        "--target-alias", "ported_totient",
        "--contract-test", CONTRACT,
        "--registry-cache", str(CACHE),
        "--git-cache", str(GIT_CACHE),
    ]
    if review:
        args.append("--license-reviewed")
    result = cli(*args, require_success=require_success)
    return result, out


def main():
    run("docker", "image", "inspect", "rust:1.99-slim")
    verify_unpublished_on_registry()
    with tempfile.TemporaryDirectory(prefix="composer-unpublished-git-") as td:
        root = Path(td)
        source, original = root / "real-unpublished-upstream", root / "real-trigram"
        clone_exact(UPSTREAM_URL, UPSTREAM_SHA, source)
        clone_exact(TARGET_URL, TARGET_SHA, original)
        assert (source / "LICENSE").is_file()
        import tomllib
        upstream_toml = tomllib.loads((source / "Cargo.toml").read_text())
        assert upstream_toml["package"]["name"] == PACKAGE
        assert "license" not in upstream_toml["package"], (
            "This test must exercise the license-file-only Git source path"
        )
        assert (source / "src/number_theory/euler_totient.rs").is_file()
        assert (original / "LICENSE").is_file()
        prepare_git_cache(original, root)

        target, baseline = make_target(original, root, "positive")
        result, output = execute(target, root, source, "positive")
        report = json.loads(result.stdout)
        assert report["offline_tests_passed"] is True
        assert report["lockfile_git_commit_verified"] is True
        assert report["source_checkout_commit_verified"] is True
        assert report["upstream_commit"] == UPSTREAM_SHA
        assert report["package"] == PACKAGE
        assert len(report["source_license_sha256"]) == 64
        assert "pub use composer_git_the_algorithms_rust::number_theory::euler_totient as ported_totient" in (
            output / "src/lib.rs"
        ).read_text()
        assert UPSTREAM_SHA in (output / "Cargo.lock").read_text()
        evidence = json.loads((output / "composer-upstream-git-evidence.json").read_text())
        assert evidence["network_disabled"] and evidence["offline_tests_passed"]
        assert evidence["cargo_lock_commit_verified"]
        assert "test result: ok" in evidence["log_tail"]
        copied_license = output / "third_party_licenses/the_algorithms_rust-git-license.txt"
        assert copied_license.read_bytes() == (source / "LICENSE").read_bytes()
        unchanged(target, baseline)
        print(f"PASS genuinely unpublished Git-only totient integration; "
              f"Docker_seconds={report['elapsed_seconds']:.2f}", flush=True)

        wrong, before = make_target(original, root, "wrong-semantic")
        response, out = execute(wrong, root, source, "wrong-semantic",
                                symbol="math::factorial::factorial",
                                require_success=False)
        assert response.returncode != 0
        assert "test result: FAILED" in response.stderr, response.stderr[-2000:]
        assert not out.exists()
        unchanged(wrong, before)
        print("PASS wrong but compilable external Git capability rejected", flush=True)

        response, out = execute(wrong, root, source, "wrong-sha",
                                sha="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                                require_success=False)
        assert response.returncode != 0
        assert "checkout HEAD differs" in response.stderr
        assert not out.exists()
        unchanged(wrong, before)
        print("PASS mismatching source SHA rejected", flush=True)

        response, out = execute(wrong, root, source, "no-license-review",
                                review=False, require_success=False)
        assert response.returncode != 0
        assert "ReviewRequired" in response.stderr
        assert not out.exists()
        unchanged(wrong, before)
        print("PASS missing explicit source/license review rejected", flush=True)

        print("EVIDENCE_JSON " + json.dumps({
            "source": UPSTREAM_URL, "sha": UPSTREAM_SHA,
            "package": PACKAGE, "published_on_crates_io": False,
            "target": TARGET_URL, "target_sha": TARGET_SHA,
            "independent_target_contract": True,
            "git_sha_matches_checkout_and_lock": True,
            "license_file_preserved": True,
            "no_host_execution": True, "docker_network_disabled": True,
            "semantic_negative_refused": True, "sha_mismatch_refused": True,
            "original_files_unchanged": True,
            "limitations": "One independently reviewed Git-only library on Linux is not "
                           "general GitHub integration, VM-level sandboxing, or Windows packaged acceptance.",
        }, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
