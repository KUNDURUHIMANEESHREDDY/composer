"""Unfamiliar public Git -> unrelated public Rust target with type adaptation.

Uses Composer's existing Cargo Git resolver and independent behavioral tests.
Neither the Git upstream algorithm nor the target crate implementation is
generated/re-written. Only a compiler-checked owned-String adapter is appended
to disposable target copies inside the verified Docker pipeline.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import tempfile

from test_pinned_public_multifile import CACHE, clone_exact, run

SOURCE_REPO = "https://github.com/kornelski/deunicode.git"
SOURCE_SHA = "cfb8552fbbdf6d1f3f996ee4f2e78ec5e482bcef"
TARGET_REPO = "https://github.com/mattgathu/slugify.git"
TARGET_SHA = "6f974c9de857ac028b4ae604423fa82c183ae006"
GIT_CACHE = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))) / "git"
CONTRACT = "tests/composer_git_string_contract.rs"
GOOD = """extern crate slugify;
use slugify::transliterate;
#[test]
fn independent_git_transliteration() {
    assert_eq!(transliterate("Æneid".to_owned()), "AEneid");
    assert_eq!(transliterate("étude".to_owned()), "etude");
    assert_eq!(transliterate("🦄☣".to_owned()), "unicorn biohazard");
    assert_eq!(transliterate("...".to_owned()), "...");
    assert_eq!(transliterate(String::new()), "");
}
"""
WRONG = """extern crate slugify;
use slugify::transliterate;
#[test]
fn wrong_but_compilable_result_must_fail() {
    assert_eq!(transliterate("Æneid".to_owned()), "Æneid");
}
"""


def cli(*args: str, require_success=True):
    return run("cargo", "run", "--quiet", "-p", "composer-cli", "--",
               *args, require_success=require_success, timeout=300)


def copy_target(source: Path, root: Path, label: str, contract=GOOD):
    target = root / label
    shutil.copytree(source, target, ignore=shutil.ignore_patterns(".git"))
    (target / "tests").mkdir(exist_ok=True)
    (target / CONTRACT).write_text(contract, encoding="utf-8")
    orig = tuple((target / name).read_bytes() for name in (
        "Cargo.toml", "src/lib.rs", CONTRACT))
    return target, orig


def unchanged(target: Path, orig):
    assert orig == tuple((target / name).read_bytes() for name in (
        "Cargo.toml", "src/lib.rs", CONTRACT))


def prefetch_git(target: Path, root: Path):
    project = root / "prefetch"
    shutil.copytree(target, project, ignore=shutil.ignore_patterns(".git"))
    cargo = project / "Cargo.toml"
    old = cargo.read_text()
    assert "[dependencies]" in old
    cargo.write_text(old.replace(
        "[dependencies]",
        "[dependencies]\n"
        f'composer_git_deunicode = {{ package = "deunicode", git = "{SOURCE_REPO}", '
        f'rev = "{SOURCE_SHA}" }}', 1
    ))
    run("cargo", "fetch", "--manifest-path", str(cargo),
        "--target", "x86_64-unknown-linux-gnu", timeout=180)
    assert (GIT_CACHE / "db").is_dir()
    assert (GIT_CACHE / "checkouts").is_dir()
    assert CACHE.is_dir()


def verify(target, output, upstream, signature="transliterate(String)->String",
           sha=SOURCE_SHA, review=True, require_success=True,
           infer=False, source_symbol="deunicode"):
    args = [
        "verify-external-git-rust", str(target),
        "--output", str(output),
        "--upstream-checkout", str(upstream),
        "--upstream-repository", SOURCE_REPO[:-4],
        "--upstream-commit", sha,
        "--package", "deunicode",
        "--public-symbol", source_symbol,
        "--target-alias", "transliterate",
        "--contract-test", CONTRACT,
        "--registry-cache", str(CACHE),
        "--git-cache", str(GIT_CACHE),
    ]
    if infer:
        assert signature is None, "inference cannot override an explicit contract"
        args.append("--infer-owned-string-signature")
    elif signature is not None:
        args += ["--target-signature", signature]
    if review:
        args.append("--license-reviewed")
    return cli(*args, require_success=require_success)


def main():
    run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-git-type-adapter-") as temp:
        root = Path(temp)
        source = root / "real-deunicode-git"
        target = root / "real-slugify"
        clone_exact(SOURCE_REPO, SOURCE_SHA, source)
        clone_exact(TARGET_REPO, TARGET_SHA, target)
        assert (source / "LICENSE").is_file()
        assert (target / "LICENSE").is_file()
        prefetch_git(target, root)
        outputs = []
        for n in range(1, 4):
            work, baseline = copy_target(target, root, f"attempt-{n}")
            dest = root / f"published-{n}"
            result = json.loads(verify(work, dest, source, signature=None, infer=True).stdout)
            assert result["offline_tests_passed"] is True
            assert result["source_checkout_commit_verified"] is True
            assert result["lockfile_git_commit_verified"] is True
            assert result["upstream_commit"] == SOURCE_SHA
            assert result["target_signature"] == "transliterate(String)->String"
            assert result["inferred_owned_string_signature"] is True
            assert len(result["source_license_sha256"]) == 64
            evidence = json.loads((dest / "composer-upstream-git-evidence.json").read_text())
            assert evidence["adapter_kind"] == "borrow_owned_strings_as_str"
            assert evidence["signature_inferred_from_pinned_source"] is True
            assert evidence["network_disabled"] is True
            assert "test result: ok" in evidence["log_tail"]
            if os.environ.get("COMPOSER_VERIFIER_RUNTIME") == "runsc":
                assert evidence["log_tail"].startswith("COMPOSER_DOCKER_RUNTIME=runsc\n"), (
                    "requested gVisor runtime not recorded in verifier evidence"
                )
                print("PASS external gVisor runsc required by verified Git import", flush=True)
            assert SOURCE_SHA in (dest / "Cargo.lock").read_text()
            assert SOURCE_SHA in (dest / "Cargo.toml").read_text()
            impl = (dest / "src/lib.rs").read_text()
            assert "composer_git_deunicode::deunicode(arg0.as_str())" in impl
            assert (dest / "third_party_licenses/deunicode-git-license.txt").is_file()
            unchanged(work, baseline)
            outputs.append({"n": n, "verified": True})
            print(f"PASS pinned Git source/type adapter {n}/3", flush=True)

        wrong, baseline = copy_target(target, root, "wrong-result", WRONG)
        dest = root / "must-not-publish-wrong"
        failed = verify(wrong, dest, source, require_success=False)
        assert failed.returncode != 0
        assert "test result: FAILED" in failed.stderr, failed.stderr[-2500:]
        assert not dest.exists()
        unchanged(wrong, baseline)
        print("PASS wrong-but-compilable semantics refused", flush=True)

        invalid, baseline = copy_target(target, root, "injection")
        dest = root / "must-not-publish-injection"
        failed = verify(invalid, dest, source,
                        signature="transliterate(String);panic!()->String",
                        require_success=False)
        assert failed.returncode != 0
        assert not dest.exists()
        unchanged(invalid, baseline)
        print("PASS unsupported API syntax refused before execution", flush=True)

        none, baseline = copy_target(target, root, "missing-adapter")
        dest = root / "must-not-publish-mismatch"
        failed = verify(none, dest, source, signature=None, require_success=False)
        assert failed.returncode != 0
        assert not dest.exists()
        unchanged(none, baseline)
        print("PASS missing type adaptation cannot be mistaken for success", flush=True)

        wrong_sha, baseline = copy_target(target, root, "wrong-source-sha")
        dest = root / "must-not-publish-wrong-sha"
        failed = verify(wrong_sha, dest, source,
                        sha="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        require_success=False)
        assert failed.returncode != 0
        assert not dest.exists()
        unchanged(wrong_sha, baseline)
        print("PASS Git commit mismatch refused", flush=True)

        unsupported, original = copy_target(target, root, "unsupported-inferred-api")
        out = root / "must-not-publish-unsupported-auto-api"
        failed = verify(unsupported, out, source, signature=None, infer=True,
                        source_symbol="deunicode_char", require_success=False)
        assert failed.returncode != 0
        assert "no unique supported public borrowed-string function" in failed.stderr
        assert not out.exists()
        unchanged(unsupported, original)
        print("PASS incompatible borrowed-string inference refused", flush=True)

        print("EVIDENCE_JSON " + json.dumps({
            "upstream_source": [SOURCE_REPO, SOURCE_SHA],
            "independent_target": [TARGET_REPO, TARGET_SHA],
            "reused_algorithm_unmodified": True,
            "signature_auto_inferred_from_verified_git_ast": True,
            "unsupported_inference_rejected": True,
            "verified_runs": outputs,
            "git_commit_matches_checkout_and_lock": True,
            "wrong_behavior_rejected": True,
            "invalid_signature_rejected": True,
            "missing_adapter_rejected": True,
            "wrong_commit_rejected": True,
            "limitations": "User-supplied reviewed signature; not open-world semantic inference. Rust compiler and independent contracts verify only exercised behavior. Docker is not VM isolation.",
        }, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
