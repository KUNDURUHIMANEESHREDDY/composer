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
    assert!((ported_jaro("".to_owned(), "".to_owned()) - 1.0).abs() < 1e-12);
    assert!((ported_jaro("same".to_owned(), "same".to_owned()) - 1.0).abs() < 1e-12);
    assert!(ported_jaro("abc".to_owned(), "xyz".to_owned()).abs() < 1e-12);
    assert!((ported_jaro("MARTHA".to_owned(), "MARHTA".to_owned()) - 17.0 / 18.0).abs() < 1e-12);
    assert!((ported_jaro("Friedrich Nietzsche".to_owned(), "Jean-Paul Sartre".to_owned()) - 0.392).abs() < 0.001);
}
"""

LEVENSHTEIN_CONTRACT = """use trigram::ported_levenshtein;
#[test]
fn independently_verify_levenshtein_not_optimal_string_alignment() {
    assert_eq!(ported_levenshtein("".to_owned(), "".to_owned()), 0);
    assert_eq!(ported_levenshtein("".to_owned(), "abc".to_owned()), 3);
    assert_eq!(ported_levenshtein("kitten".to_owned(), "sitting".to_owned()), 3);
    assert_eq!(ported_levenshtein("ab".to_owned(), "ba".to_owned()), 2);
    assert_eq!(ported_levenshtein("same".to_owned(), "same".to_owned()), 0);
}
"""

AMBIGUOUS_CONTRACT = """use trigram::ported_levenshtein;
#[test]
fn same_signature_insufficient_contract_must_not_choose_automatically() {
    assert_eq!(ported_levenshtein("".to_owned(), "".to_owned()), 0);
    assert_eq!(ported_levenshtein("same".to_owned(), "same".to_owned()), 0);
    assert_eq!(ported_levenshtein("a".to_owned(), "a".to_owned()), 0);
    assert_eq!(ported_levenshtein("abc".to_owned(), "abc".to_owned()), 0);
}
"""

def cli(*args: str, require_success=True):
    return existing.cli(*args, require_success=require_success)

def fixture(target: Path, root: Path, label: str, conflict=False, contract=GOOD):
    work, _ = existing.make_target(target, root, label, conflict=conflict)
    (work / CONTRACT).write_text(contract, encoding="utf-8")
    before = [(work / path).read_bytes() for path in ("Cargo.toml", "src/lib.rs", CONTRACT)]
    return work, before

def verify(target: Path, output: Path, upstream: Path, symbol="jaro",
           sha=SOURCE_SHA, require_success=True, infer=True,
           alias="ported_jaro", typed_alias=False, alternatives=()):
    args = [
        "verify-external-git-rust", str(target),
        "--output", str(output),
        "--upstream-checkout", str(upstream),
        "--upstream-repository", SOURCE_REPO[:-4],
        "--upstream-commit", sha,
        "--package", "strsim",
        "--public-symbol", symbol,
        "--target-alias", alias,
        "--contract-test", CONTRACT,
        "--registry-cache", str(existing.CACHE),
        "--git-cache", str(GIT_CACHE),
        "--license-reviewed",
    ]
    if infer:
        args.append("--infer-typed-signature" if typed_alias else "--infer-owned-string-signature")
    for candidate in alternatives:
        args += ["--alternative-public-symbol", candidate]
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

        # New held-out type: the unchanged upstream Levenshtein returns usize.
        # The previous adapter only handled f32/f64/bool/fixed-width integers.
        for n in range(1, 4):
            work, before = fixture(
                target, root, f"owned-usize-{n}", contract=LEVENSHTEIN_CONTRACT
            )
            out = root / f"published-levenshtein-{n}"
            actual = json.loads(
                verify(
                    work, out, source, symbol="levenshtein",
                    alias="ported_levenshtein", typed_alias=True,
                ).stdout
            )
            assert actual["offline_tests_passed"] is True
            assert actual["source_checkout_commit_verified"] is True
            assert actual["lockfile_git_commit_verified"] is True
            assert actual["target_signature"] == "ported_levenshtein(String,String)->usize"
            adapter = (out / "src/lib.rs").read_text()
            assert "pub fn ported_levenshtein(arg0: ::std::string::String, arg1: ::std::string::String) -> usize" in adapter
            assert "composer_git_strsim::levenshtein(arg0.as_str(), arg1.as_str())" in adapter
            evidence = json.loads((out / "composer-upstream-git-evidence.json").read_text())
            assert evidence["inference_source_parameters"] == ["&str", "&str"]
            assert evidence["adapter_kind"] == "borrow_owned_strings_as_str"
            assert evidence["network_disabled"] is True
            assert SOURCE_SHA in (out / "Cargo.lock").read_text()
            existing.assert_unchanged(work, before)
            print(f"PASS upstream Levenshtein usize API inference {n}/3", flush=True)

        wrong_target, baseline = fixture(
            target, root, "wrong-usize-semantics", contract=LEVENSHTEIN_CONTRACT
        )
        refused = root / "must-not-publish-osa-as-levenshtein"
        wrong_result = verify(
            wrong_target, refused, source, symbol="osa_distance",
            alias="ported_levenshtein", typed_alias=True, require_success=False,
        )
        assert wrong_result.returncode != 0
        assert "test result: FAILED" in wrong_result.stderr, wrong_result.stderr[-1800:]
        assert not refused.exists()
        existing.assert_unchanged(wrong_target, baseline)
        print("PASS type-identical but semantically wrong distance algorithm rejected", flush=True)

        # Unlike type-only inference, select among two reviewed candidates
        # by actually running the target's pre-authored behavioral contract.
        # First candidate has the same Rust signature but is semantically wrong.
        for n in range(1, 4):
            work, before = fixture(
                target, root, f"candidate-unique-{n}", contract=LEVENSHTEIN_CONTRACT
            )
            out = root / f"unique-contract-selected-{n}"
            data = json.loads(
                verify(
                    work, out, source, symbol="osa_distance",
                    alternatives=("levenshtein",), alias="ported_levenshtein",
                    typed_alias=True,
                ).stdout
            )
            assert data["public_symbol"] == "levenshtein"
            assert data["offline_tests_passed"] is True
            assert data["lockfile_git_commit_verified"] is True
            assert data["source_checkout_commit_verified"] is True
            evidence = json.loads((out / "composer-upstream-git-evidence.json").read_text())
            assert evidence["contract_based_candidate_selection"] is True
            assert evidence["reviewed_candidate_count"] == 2
            assert evidence["candidate_attempts"] == [
                {"symbol": "osa_distance", "verified": False},
                {"symbol": "levenshtein", "verified": True},
            ]
            assert evidence["semantic_equivalence_proven"] is False
            assert evidence["network_disabled"] is True
            assert not any(out.parent.glob(".composer-candidate-selection-*"))
            existing.assert_unchanged(work, before)
            print(f"PASS automatic unique contract-verified source selection {n}/3", flush=True)

        # A too-weak contract passes both Levenshtein and OSA. This is
        # ambiguous evidence, so refuse publication rather than guessing.
        weak, baseline = fixture(
            target, root, "ambiguous-behavior", contract=AMBIGUOUS_CONTRACT
        )
        ambiguous_output = root / "must-not-publish-ambiguous-source"
        ambiguous = verify(
            weak, ambiguous_output, source, symbol="levenshtein",
            alternatives=("osa_distance",), alias="ported_levenshtein",
            typed_alias=True, require_success=False,
        )
        assert ambiguous.returncode != 0
        assert "not unique: 2 out of 2" in ambiguous.stderr, ambiguous.stderr[-2200:]
        assert not ambiguous_output.exists()
        assert not any(root.glob(".composer-candidate-selection-*"))
        existing.assert_unchanged(weak, baseline)
        print("PASS ambiguous weak contract refuses automatic publication", flush=True)

        # A genuinely wrong pair must never masquerade as one success.
        missing, baseline = fixture(
            target, root, "no-behavioral-match", contract=LEVENSHTEIN_CONTRACT
        )
        no_match_output = root / "must-not-publish-no-matching-source"
        none_match = verify(
            missing, no_match_output, source, symbol="osa_distance",
            alternatives=("damerau_levenshtein",),
            alias="ported_levenshtein", typed_alias=True, require_success=False,
        )
        assert none_match.returncode != 0
        assert "not unique: 0 out of 2" in none_match.stderr, none_match.stderr[-2200:]
        assert not no_match_output.exists()
        assert not any(root.glob(".composer-candidate-selection-*"))
        existing.assert_unchanged(missing, baseline)
        print("PASS zero behavioral matches refuse automatic publication", flush=True)

        print("EVIDENCE_JSON " + json.dumps({
            "source": [SOURCE_REPO, SOURCE_SHA],
            "target": [TARGET_REPO, TARGET_SHA],
            "upstream_algorithm_reimplemented": False,
            "numeric_return_type_inferred_from_pinned_upstream_AST": True,
            "usize_levenshtein_accepted_3_times": True,
            "same_signature_wrong_distance_algorithm_rejected": True,
            "contract_based_reviewed_symbol_selection_passed_3_times": True,
            "ambiguous_weak_contract_rejected": True,
            "zero_match_rejected": True,
            "verified_runs": recorded,
            "wrong_semantics_rejected": True,
            "generic_source_rejected": True,
            "no_adapter_rejected": True,
            "original_targets_unchanged": True,
            "limits": "Only an explicitly chosen public &str function with an allowed scalar or String return; no general semantic inference. Cargo lock, licensing and independently authored tests remain required. Docker is not VM-grade containment.",
        }, sort_keys=True), flush=True)

if __name__ == "__main__":
    main()
