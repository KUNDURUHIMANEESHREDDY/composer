"""Check real numeric-output API inference against independent pinned Git source.

Only the adapter is generated. The Jaro algorithm comes from
rapidfuzz/strsim-rs; the target is unrelated ijt/trigram.
"""
from __future__ import annotations

import json
import shutil
import tempfile
from pathlib import Path

import test_external_git_source_public as existing

SOURCE_REPO = existing.SOURCE_REPO
SOURCE_SHA = existing.SOURCE_SHA
TARGET_REPO = existing.TARGET_REPO
TARGET_SHA = existing.TARGET_SHA
GIT_CACHE = existing.GIT_CACHE
CONTRACT = existing.CONTRACT

GOOD = """use trigram::ported_jaro;
#[test]
fn independently_verify_owned_string_jaro() {
    let cases = [
        ("", "", 1.0),
        ("same", "same", 1.0),
        ("abc", "xyz", 0.0),
        ("MARTHA", "MARHTA", 17.0 / 18.0),
        ("Friedrich Nietzsche", "Jean-Paul Sartre", 0.3918859649122807),
    ];
    for (a, b, value) in cases {
        let actual = ported_jaro(a.to_owned(), b.to_owned());
        assert!((actual - value).abs() < 1e-6, "case {a} vs {b}: {actual}");
    }
}
"""

def cli(*args: str, require_success=True):
    return existing.cli(*args, require_success=require_success)

def fixture(target: Path, root: Path, label: str, conflict=False):
    work, _ = existing.make_target(target, root, label, conflict=conflict)
    (work / CONTRACT).write_text(GOOD, encoding="utf-8")
    before = [(work / path).read_bytes() for path in ("Cargo.toml", "src/lib.rs", CONTRACT)]
    return work, before

def verify(target: Path, output: Path, upstream: Path, symbol="jaro",
           sha=SOURCE_SHA, require_success=True, infer=True):
    args = [
        "verify-external-git-rust", str(target),
        "--output", str(output),
        "--upstream-checkout", str(upstream),
        "--upstream-repository", SOURCE_REPO[:-4],
        "--upstream-commit", sha,
        "--package", "strsim",
        "--public-symbol", symbol,
        "--target-alias", "ported_jaro",
        "--contract-test", CONTRACT,
        "--registry-cache", str(existing.CACHE),
        "--git-cache", str(GIT_CACHE),
        "--license-reviewed",
    ]
    if infer:
        args.append("--infer-owned-string-signature")
    return cli(*args, require_success=require_success)

def main():
    existing.run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-owned-scalar-inference-") as temp:
        root = Path(temp)
        source = root / "third-party-strsim"
        target = root / "third-party-trigram"
        existing.clone_exact(SOURCE_REPO, SOURCE_SHA, source)
        existing.clone_exact(TARGET_REPO, TARGET_SHA, target)
        assert (source / "LICENSE").is_file()
        assert (target / "LICENSE").is_file()
        existing.prefetch_git(target, root)

        recorded = []
        for n in range(1, 4):
            work, before = fixture(target, root, f"owned-f64-{n}", conflict=n == 2)
            out = root / f"published-{n}"
            result = json.loads(verify(work, out, source).stdout)
            assert result["offline_tests_passed"] is True
            assert result["lockfile_git_commit_verified"] is True
            assert result["source_checkout_commit_verified"] is True
            assert result["inferred_owned_string_signature"] is True
            assert result["target_signature"] == "ported_jaro(String,String)->f64"
            assert result["upstream_commit"] == SOURCE_SHA
            evidence = json.loads((out / "composer-upstream-git-evidence.json").read_text())
            assert evidence["signature_inferred_from_pinned_source"] is True
            assert evidence["adapter_kind"] == "borrow_owned_strings_as_str"
            assert evidence["network_disabled"] is True
            src = (out / "src/lib.rs").read_text()
            assert "pub fn ported_jaro(arg0: ::std::string::String, arg1: ::std::string::String) -> f64" in src
            assert "composer_git_strsim::jaro(arg0.as_str(), arg1.as_str())" in src
            assert SOURCE_SHA in (out / "Cargo.lock").read_text()
            assert (out / "third_party_licenses/strsim-git-license.txt").is_file()
            existing.assert_unchanged(work, before)
            if n == 2:
                assert '=0.10.0' in (out / "Cargo.toml").read_text()
            print(f"PASS independently tested external Git numeric API inference {n}/3", flush=True)
            recorded.append({"run": n, "verified": True, "conflicting_existing_dependency": n == 2})

        work, before = fixture(target, root, "wrong-semantic")
        out = root / "must-not-publish-wrong-semantic"
        wrong = verify(work, out, source, symbol="jaro_winkler", require_success=False)
        assert wrong.returncode != 0
        assert "test result: FAILED" in wrong.stderr, wrong.stderr[-2000:]
        assert not out.exists()
        existing.assert_unchanged(work, before)
        print("PASS wrong-but-compiling numeric semantics rejected", flush=True)

        work, before = fixture(target, root, "unsupported-generic")
        out = root / "must-not-publish-generic"
        invalid = verify(work, out, source, symbol="generic_jaro", require_success=False)
        assert invalid.returncode != 0
        assert "no unique supported public borrowed-string function" in invalid.stderr, invalid.stderr[-2000:]
        assert not out.exists()
        existing.assert_unchanged(work, before)
        print("PASS generic upstream API rejected before execution", flush=True)

        work, before = fixture(target, root, "no-owned-adapter")
        out = root / "must-not-publish-no-adapter"
        invalid = verify(work, out, source, infer=False, require_success=False)
        assert invalid.returncode != 0
        assert "test result: FAILED" in invalid.stderr or "error[E0308]" in invalid.stderr
        assert not out.exists()
        existing.assert_unchanged(work, before)
        print("PASS no-adapter type mismatch refused", flush=True)

        print("EVIDENCE_JSON " + json.dumps({
            "source": [SOURCE_REPO, SOURCE_SHA],
            "target": [TARGET_REPO, TARGET_SHA],
            "upstream_algorithm_reimplemented": False,
            "numeric_return_type_inferred_from_pinned_upstream_AST": True,
            "verified_runs": recorded,
            "wrong_semantics_rejected": True,
            "generic_source_rejected": True,
            "no_adapter_rejected": True,
            "original_targets_unchanged": True,
            "limits": "Only an explicitly chosen public &str function with an allowed scalar or String return; no general semantic inference. Cargo lock, licensing and independently authored tests remain required. Docker is not VM-grade containment.",
        }, sort_keys=True), flush=True)

if __name__ == "__main__":
    main()
