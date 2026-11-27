"""Reuse a previously untested third-party repository with a typed API bridge.

Source: kornelski/deunicode (BSD-3-Clause, published crate).
Target: mattgathu/slugify (MIT, unrelated, external Rust library).
The transliteration algorithm stays upstream; Composer only generates a safe
String -> &str adapter, compiles it in offline Docker and tests real semantics.
"""
from __future__ import annotations

import json
from pathlib import Path
import shutil
import tempfile

from test_pinned_public_multifile import CACHE, clone_exact, run

UPSTREAM = ("https://github.com/kornelski/deunicode.git",
            "cfb8552fbbdf6d1f3f996ee4f2e78ec5e482bcef")
TARGET = ("https://github.com/mattgathu/slugify.git",
          "6f974c9de857ac028b4ae604423fa82c183ae006")
CATALOG = Path(__file__).resolve().parents[1] / "examples/approved-external-capabilities.json"
CONTRACT = "tests/composer_external_transliteration_contract.rs"
VALID = """extern crate slugify;
use slugify::transliterate;
#[test]
fn independent_external_transliteration() {
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
fn incorrect_semantics_must_not_publish() {
    assert_eq!(transliterate("Æneid".to_owned()), "Æneid");
}
"""


def cli(*args, require_success=True):
    return run("cargo", "run", "--quiet", "-p", "composer-cli", "--",
               *args, timeout=300, require_success=require_success)


def stage(original: Path, root: Path, label: str, contract: str):
    work = root / label
    shutil.copytree(original, work, ignore=shutil.ignore_patterns(".git"))
    (work / "tests").mkdir(exist_ok=True)
    (work / CONTRACT).write_text(contract, encoding="utf-8")
    baseline = tuple((work / name).read_bytes()
                     for name in ("Cargo.toml", "src/lib.rs", CONTRACT))
    return work, baseline


def unchanged(work: Path, baseline) -> None:
    assert baseline == tuple((work / name).read_bytes()
                             for name in ("Cargo.toml", "src/lib.rs", CONTRACT))


def compose(work: Path, output: Path, *, signature="transliterate(String)->String",
            require_success=True):
    args = [
        "compose-external-rust", "unicode to ascii transliteration",
        str(work), "--catalog", str(CATALOG), "--output", str(output),
        "--target-alias", "transliterate",
        "--contract-test", CONTRACT, "--registry-cache", str(CACHE),
        "--exclude-owner", "KUNDURUHIMANEESHREDDY",
        "--license-reviewed",
    ]
    if signature is not None:
        args.extend(["--target-signature", signature])
    return cli(*args, require_success=require_success)


def main():
    run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-heldout-string-adapter-") as temp:
        root = Path(temp)
        upstream = root / "original-deunicode"
        target = root / "original-slugify"
        clone_exact(*UPSTREAM, upstream)
        clone_exact(*TARGET, target)
        assert (upstream / "LICENSE").is_file()
        assert (target / "LICENSE").is_file()
        # Use Cargo's existing registry acquisition, not newly authored
        # dependency downloads or a new transliteration implementation.
        prefetch = root / "prefetch"
        shutil.copytree(target, prefetch, ignore=shutil.ignore_patterns(".git"))
        manifest = prefetch / "Cargo.toml"
        manifest.write_text(manifest.read_text() + '\n[dev-dependencies]\ndeunicode = "=1.6.2"\n')
        run("cargo", "fetch", "--manifest-path", str(manifest),
            "--target", "x86_64-unknown-linux-gnu", timeout=180)

        outcomes = []
        for n in range(1, 4):
            work, baseline = stage(target, root, f"success-{n}", VALID)
            published = root / f"published-{n}"
            result = json.loads(compose(work, published).stdout)
            assert result["selection"]["package"] == "deunicode"
            assert result["selection"]["approved_pinned_version"] == "1.6.2"
            evidence = result["integration"]
            assert evidence["target_signature"] == "transliterate(String)->String"
            assert evidence["offline_tests_passed"]
            assert evidence["original_target_unchanged"]
            assert len(evidence["registry_checksum"]) == 64
            log = json.loads((published / "composer-upstream-crate-evidence.json").read_text())
            assert log["adapter_kind"] == "borrow_owned_strings_as_str"
            assert log["network_disabled"]
            generated = (published / "src/lib.rs").read_text()
            assert "deunicode::deunicode(arg0.as_str())" in generated
            assert "pub fn transliterate(arg0: ::std::string::String)" in generated
            assert (published / "LICENSE").is_file()
            unchanged(work, baseline)
            outcomes.append({"iteration": n, "verified": True})
            print(f"PASS independent third-party API adaptation {n}/3", flush=True)

        bad, original = stage(target, root, "incorrect-behavior", WRONG)
        out = root / "must-not-publish-behavior"
        error = compose(bad, out, require_success=False)
        assert error.returncode != 0
        assert "test result: FAILED" in error.stderr, error.stderr[-2100:]
        assert not out.exists()
        unchanged(bad, original)
        print("PASS wrong-but-compilable transliteration contract rejected", flush=True)

        invalid, baseline = stage(target, root, "malformed-signature", VALID)
        out = root / "must-not-publish-invalid-signature"
        error = compose(invalid, out, signature="transliterate(String);panic!()->String",
                        require_success=False)
        assert error.returncode != 0
        assert not out.exists()
        unchanged(invalid, baseline)
        print("PASS unreviewable adapter type/signature rejected", flush=True)

        without, baseline = stage(target, root, "without-adaptation", VALID)
        out = root / "must-not-publish-mismatched-api"
        error = compose(without, out, signature=None, require_success=False)
        assert error.returncode != 0
        assert not out.exists()
        unchanged(without, baseline)
        print("PASS incompatible original API not mistaken for adapted completion", flush=True)

        print("EVIDENCE_JSON " + json.dumps({
            "source": {"repository": UPSTREAM[0], "sha": UPSTREAM[1]},
            "unfamiliar_target": {"repository": TARGET[0], "sha": TARGET[1]},
            "original_upstream_algorithm_reimplemented": False,
            "verified_owned_string_adaptations": outcomes,
            "independent_behavioral_rejection": True,
            "type_mismatch_rejected": True,
            "injection_rejected": True,
            "remaining": "Reviewed signature supplied; not open-ended automatic API inference. Cargo validates types; tests are independent but not complete semantic equivalence. Source Git commit does not cryptographically match registry artifact.",
        }, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
