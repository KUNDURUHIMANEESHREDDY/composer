"""Controlled end-to-end acceptance checks for offline Rust composition.

Runs only on an explicitly provisioned development/CI machine with Docker.
The generated source is original, synthetic fixture code, not a third-party repo.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
CARGO_CACHE = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))) / "registry"


def command(*args: str, expect_success: bool = True) -> str:
    result = subprocess.run(
        list(args), cwd=ROOT, capture_output=True, text=True,
        encoding="utf-8", errors="replace", timeout=300,
    )
    if expect_success and result.returncode:
        raise AssertionError(
            f"Command failed ({result.returncode}): {args}\n"
            f"stdout:\n{result.stdout[-6000:]}\nstderr:\n{result.stderr[-6000:]}"
        )
    if not expect_success and result.returncode == 0:
        raise AssertionError(f"Unexpected success: {args}")
    return result.stdout


def cli(*args: str, expect_success: bool = True) -> str:
    return command("cargo", "run", "--quiet", "-p", "composer-cli", "--",
                   *args, expect_success=expect_success)


def make_repo(base: Path, name: str, source: str, deps: str = "") -> None:
    (base / "src").mkdir(parents=True)
    (base / "Cargo.toml").write_text(
        f'[package]\nname="{name}"\nversion="0.1.0"\nedition="2024"'
        f'\nlicense="MIT"\n[dependencies]\n{deps}',
        encoding="utf-8",
    )
    (base / "src" / "feature.rs").write_text(source, encoding="utf-8")
    (base / "LICENSE").write_text("MIT fixture, for controlled tests only", encoding="utf-8")


def make_capsule(root: Path, source: Path, symbol: str, capability: str) -> Path:
    graph = json.loads(cli("analyze", str(source)))
    seeds = [item["id"] for item in graph["symbols"] if item["name"] == symbol]
    if len(seeds) != 1:
        raise AssertionError(f"Unexpected seed candidates for {symbol}: {seeds}")
    capsule = root / f"capsule-{capability}"
    cli("capsule", str(source), "--capability-id", capability,
        "--description", "Controlled Rust e2e feature",
        "--seed-symbol", seeds[0], "--output", str(capsule))
    return capsule


def verify_nested_modules(root: Path, target: Path) -> None:
    """Real CLI capsule closure -> three-file transplant -> Docker tests -> rollback."""
    source = root / "source-nested"
    (source / "src/math").mkdir(parents=True)
    (source / "Cargo.toml").write_text(
        '[package]\nname="nested_source"\nversion="0.1.0"\nedition="2024"'
        '\nlicense="MIT"\n[dependencies]\nserde_json = "1"\n', encoding="utf-8"
    )
    (source / "LICENSE").write_text("MIT fixture for local verification", encoding="utf-8")
    (source / "src/lib.rs").write_text(
        'pub mod math;\n'
        'pub fn calculate(n: u32) -> u32 { crate::math::double(n) }\n'
        '#[cfg(test)] mod tests { #[test] fn calculates() { '
        'assert_eq!(super::calculate(21), 42); } }\n', encoding="utf-8"
    )
    (source / "src/math.rs").write_text(
        'pub mod util;\n'
        'pub fn double(n: u32) -> u32 { self::util::scale(n) }\n'
        'pub fn encode(n: u32) -> String { serde_json::to_string(&double(n)).unwrap() }\n'
        '#[cfg(test)] mod tests { #[test] fn encodes() { '
        'assert_eq!(super::encode(21), "42"); } }\n', encoding="utf-8"
    )
    (source / "src/math/util.rs").write_text(
        'pub fn scale(n: u32) -> u32 { n * 2 }\n'
        '#[cfg(test)] mod tests { #[test] fn scales() { '
        'assert_eq!(super::scale(4), 8); } }\n', encoding="utf-8"
    )
    capsule = make_capsule(root, source, "calculate", "nested-calculator")
    manifest = json.loads((capsule / "manifest.json").read_text())
    assert {x.replace("\\", "/") for x in manifest["included_source_files"]} == {
        "src/lib.rs", "src/math.rs", "src/math/util.rs"
    }, "Analyzer must follow bare mod declarations inside src/math.rs"
    output = root / "verified-nested"
    original = (target / "src/lib.rs").read_bytes()
    result = json.loads(cli(
        "verify-rust", str(capsule), str(target),
        "--output", str(output), "--license-reviewed",
        "--registry-cache", str(CARGO_CACHE),
    ))
    assert result["isolated_tests_passed"] is True
    for file in ("mod.rs", "math.rs", "math/util.rs"):
        assert (output / "src/composer_nested_calculator" / file).is_file(), file
    assert "crate::composer_nested_calculator::math::double" in (
        output / "src/composer_nested_calculator/mod.rs").read_text()
    assert "serde_json = 1" in result["new_dependencies"]
    assert (target / "src/lib.rs").read_bytes() == original
    assert not (target / "src/composer_nested_calculator").exists()
    attributions = json.loads((output / "composer-source-attributions.json").read_text())
    assert len(attributions) == 1
    assert (output / attributions[0]["license_documents"][0]["published_path"]).is_file()
    assert "test result: ok" in json.loads(
        (output / "composer-build-evidence.json").read_text()
    )["log_tail"]
    print("PASS nested Rust modules: 3-file dependency closure, crate path rewrite,"
          " offline serde_json, original unchanged")

    # Contract test proves the alias resolves from the target's public crate
    # API. This target cannot compile its integration test without the alias.
    target_api = root / "target-nested-api"
    shutil.copytree(target, target_api)
    (target_api / "tests").mkdir()
    (target_api / "tests/api_contract.rs").write_text(
        "use composed_target::adapted_scale;\n"
        "#[test] fn adapted_public_api_works() { "
        "assert_eq!(adapted_scale(21), 42); }\n",
        encoding="utf-8"
    )
    output_api = root / "verified-nested-api"
    adapted = json.loads(cli(
        "verify-rust", str(capsule), str(target_api),
        "--output", str(output_api), "--license-reviewed",
        "--registry-cache", str(CARGO_CACHE),
        "--api-alias", "math::double=adapted_scale",
    ))
    assert adapted["isolated_tests_passed"] is True
    assert adapted["verified_api_aliases"] == ["math::double=adapted_scale"]
    assert "pub use crate::composer_nested_calculator::math::double as adapted_scale;" in (
        output_api / "src/lib.rs"
    ).read_text()
    assert (target_api / "src/lib.rs").read_bytes() == original
    assert json.loads((output_api / "composer-build-evidence.json").read_text())[
        "verified_api_aliases"
    ] == ["math::double=adapted_scale"]
    print("PASS compiler-verified API facade: source symbol adapted to target contract")

    # Compiler infers only Into conversions; no lossy casts or source edits.
    # u16 -> source u32 -> target u64 is infallible for primitive types.
    target_signature = root / "target-nested-signature"
    shutil.copytree(target, target_signature)
    (target_signature / "tests").mkdir()
    (target_signature / "tests/signature_contract.rs").write_text(
        "use composed_target::adapted_scale;\n"
        "#[test] fn target_signature_works() { "
        "let value: u64 = adapted_scale(21_u16); "
        "assert_eq!(value, 42_u64); }\n",
        encoding="utf-8",
    )
    output_signature = root / "verified-nested-signature"
    signature = json.loads(cli(
        "verify-rust", str(capsule), str(target_signature),
        "--output", str(output_signature), "--license-reviewed",
        "--registry-cache", str(CARGO_CACHE),
        "--api-signature", "math::double=adapted_scale(u16)->u64",
    ))
    assert signature["isolated_tests_passed"] is True
    assert signature["verified_api_signatures"] == [
        "math::double=adapted_scale(u16)->u64"
    ]
    generated = (output_signature / "src/lib.rs").read_text()
    assert "pub fn adapted_scale(arg0: u16) -> u64" in generated
    assert "let adapted_result: u64" in generated
    assert (target_signature / "src/lib.rs").read_bytes() == original
    assert json.loads((output_signature / "composer-build-evidence.json").read_text())[
        "verified_api_signatures"
    ] == ["math::double=adapted_scale(u16)->u64"]
    print("PASS compiler-inferred API signature: u16 to u32 input, u32 to u64 output")

    # Connect the read-only recommender to the *existing* verifier only
    # after explicit confirmation of the exact current rule and an existing
    # independent behavioral contract test. Replace the placeholder body,
    # never add a duplicate function declaration.
    target_reviewed = root / "target-reviewed-suggestion"
    shutil.copytree(target, target_reviewed)
    reviewed_lib = target_reviewed / "src/lib.rs"
    reviewed_lib.write_text(
        reviewed_lib.read_text() +
        "\npub fn adapted_double(n: u16) -> u64 { todo!() }\n",
        encoding="utf-8",
    )
    (target_reviewed / "tests").mkdir()
    (target_reviewed / "tests/reviewed_contract.rs").write_text(
        "use composed_target::adapted_double;\n"
        "#[test] fn checks_reviewed_mapping() {\n"
        "    assert_eq!(adapted_double(21_u16), 42_u64);\n"
        "    assert_eq!(adapted_double(4_u16), 8_u64);\n"
        "}\n",
        encoding="utf-8",
    )
    original_reviewed = reviewed_lib.read_bytes()
    proposed = json.loads(cli(
        "suggest-rust-api", str(capsule), str(target_reviewed),
    ))
    candidates = [item for item in proposed["requirements"]
                  if item["target_function"] == "adapted_double"]
    assert len(candidates) == 1
    reviewed_rule = candidates[0]["best_suggestion"]["suggested_cli_rule"]
    assert reviewed_rule == "math::double=adapted_double(u16)->u64"

    verified_reviewed = root / "verified-reviewed-suggestion"
    reviewed_result = json.loads(cli(
        "verify-rust", str(capsule), str(target_reviewed),
        "--output", str(verified_reviewed), "--license-reviewed",
        "--registry-cache", str(CARGO_CACHE),
        "--reviewed-api-rule", reviewed_rule,
        "--contract-test", "tests/reviewed_contract.rs",
    ))
    assert reviewed_result["isolated_tests_passed"] is True
    assert reviewed_result["reviewed_api_rule"] == reviewed_rule
    assert reviewed_result["reviewed_contract_test"] == "tests/reviewed_contract.rs"
    output_lib = (verified_reviewed / "src/lib.rs").read_text()
    assert output_lib.count("pub fn adapted_double") == 1
    assert "crate::composer_nested_calculator::math::double" in output_lib
    assert "pub fn adapted_double(n: u16) -> u64 { todo!() }" not in output_lib
    assert reviewed_lib.read_bytes() == original_reviewed
    evidence = json.loads((verified_reviewed / "composer-build-evidence.json").read_text())
    assert evidence["reviewed_api_rule"] == reviewed_rule
    assert evidence["reviewed_contract_test"] == "tests/reviewed_contract.rs"
    print("PASS reviewed suggestion -> existing stub -> target contract -> Docker tests")

    for index, (flags, reason) in enumerate([
        (["--reviewed-api-rule", "math::double=adapted_double(u32)->u64",
          "--contract-test", "tests/reviewed_contract.rs"], "stale rule"),
        (["--reviewed-api-rule", reviewed_rule], "missing test declaration"),
        (["--reviewed-api-rule", reviewed_rule,
          "--contract-test", "../reviewed_contract.rs"], "traversal"),
        (["--reviewed-api-rule", reviewed_rule,
          "--contract-test", "tests/reviewed_contract.rs",
          "--api-alias", "math::double=other"], "mixed modes"),
    ]):
        output = root / f"reviewed-must-reject-{index}"
        cli(
            "verify-rust", str(capsule), str(target_reviewed),
            "--output", str(output), "--license-reviewed",
            "--registry-cache", str(CARGO_CACHE),
            *flags, expect_success=False,
        )
        assert not output.exists(), f"{reason}: invalid reviewed integration published"
        assert reviewed_lib.read_bytes() == original_reviewed
    print("PASS reviewed mapping refused stale rules, missing tests, traversal and mixed modes")

    bad_signature = root / "must-reject-incompatible-signature"
    cli(
        "verify-rust", str(capsule), str(target),
        "--output", str(bad_signature), "--license-reviewed",
        "--registry-cache", str(CARGO_CACHE),
        "--api-signature", "math::double=adapted_scale(String)->String",
        expect_success=False,
    )
    assert not bad_signature.exists(), "Incompatible signature must not publish"
    print("PASS incompatible source-to-target type conversion rejected")

    bad_api_output = root / "must-reject-unsafe-alias"
    cli(
        "verify-rust", str(capsule), str(target),
        "--output", str(bad_api_output), "--license-reviewed",
        "--registry-cache", str(CARGO_CACHE),
        "--api-alias", "math::double=bad;inject",
        expect_success=False
    )
    assert not bad_api_output.exists()
    print("PASS malformed API alias rejected without publishing output")

    broken = root / "source-nested-broken"
    shutil.copytree(source, broken)
    leaf = broken / "src/math/util.rs"
    leaf.write_text(
        leaf.read_text().replace(
            "assert_eq!(super::scale(4), 8)",
            "assert_eq!(super::scale(4), 9)"
        ), encoding="utf-8"
    )
    broken_capsule = make_capsule(root, broken, "calculate", "nested-broken")
    rejected = root / "nested-must-not-publish"
    cli(
        "verify-rust", str(broken_capsule), str(target),
        "--output", str(rejected), "--license-reviewed",
        "--registry-cache", str(CARGO_CACHE),
        expect_success=False
    )
    assert not rejected.exists(), "A failing nested test must not publish output"
    assert (target / "src/lib.rs").read_bytes() == original
    print("PASS multi-file rollback: failing nested test leaves no published output")


def verify_cargo_dependency_features(root: Path, target: Path) -> None:
    """Used, versioned serde/derive feature gets added without altering target version."""
    source = root / "source-feature-flags"
    make_repo(source, "feature_flags", """
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
pub struct Record { pub value: u32 }
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn serializes() {
        let x = Record { value: 42 };
        let json = serde_json::to_string(&x).unwrap();
        assert_eq!(serde_json::from_str::<Record>(&json).unwrap(), x);
    }
}
""", 'serde = { version = "1", features = ["derive"] }\nserde_json = "1"\n')
    capsule = make_capsule(root, source, "Record", "serde-features")
    target_copy = root / "target-feature-flags"
    shutil.copytree(target, target_copy)
    with (target_copy / "Cargo.toml").open("a", encoding="utf-8") as out:
        out.write('serde = "1"\n')
    original = (target_copy / "Cargo.toml").read_bytes()
    output = root / "verified-features"
    result = json.loads(cli(
        "verify-rust", str(capsule), str(target_copy),
        "--output", str(output), "--license-reviewed",
        "--registry-cache", str(CARGO_CACHE),
    ))
    assert result["isolated_tests_passed"] is True
    assert result["dependency_feature_changes"] == ["serde: enabled [derive]"]
    assert "serde_json = 1" in result["new_dependencies"]
    manifest = (output / "Cargo.toml").read_text()
    assert "derive" in manifest
    assert "serde" in manifest
    assert (target_copy / "Cargo.toml").read_bytes() == original
    assert json.loads((output / "composer-build-evidence.json").read_text())[
        "dependency_feature_changes"
    ] == ["serde: enabled [derive]"]
    print("PASS Cargo feature compatibility: serde/derive adopted, target version retained")

    # Refuse git/path/renamed dependencies even when they contain a version.
    invalid = root / "source-unsafe-feature"
    shutil.copytree(source, invalid)
    manifest_path = invalid / "Cargo.toml"
    manifest_path.write_text(manifest_path.read_text().replace(
        'version = "1", features = ["derive"]',
        'version = "1", git = "https://example.invalid/foreign"',
    ), encoding="utf-8")
    invalid_capsule = make_capsule(root, invalid, "Record", "unsafe-feature")
    rejected = root / "must-reject-foreign-dependency"
    cli(
        "verify-rust", str(invalid_capsule), str(target_copy),
        "--output", str(rejected), "--license-reviewed",
        "--registry-cache", str(CARGO_CACHE),
        expect_success=False,
    )
    assert not rejected.exists()
    assert (target_copy / "Cargo.toml").read_bytes() == original
    print("PASS git dependency rejected without target mutation or publication")


def main() -> None:
    command("docker", "image", "inspect", "rust:1.99-slim")
    if not CARGO_CACHE.is_dir():
        raise RuntimeError(f"Cargo registry cache unavailable: {CARGO_CACHE}")
    with tempfile.TemporaryDirectory(prefix="composer-ci-e2e-") as root_name:
        root = Path(root_name)
        a, b, target = root / "source-a", root / "source-b", root / "target"
        make_repo(a, "source_a", '''
pub fn twice(n: u32) -> u32 { n * 2 }
#[cfg(test)]
mod tests { #[test] fn doubles() { assert_eq!(super::twice(21), 42); } }
''')
        make_repo(b, "source_b", '''
pub fn as_json() -> String { serde_json::to_string(&42).unwrap() }
#[cfg(test)]
mod tests { #[test] fn encodes() { assert_eq!(super::as_json(), "42"); } }
''', 'serde_json = "1"\n')
        (target / "src").mkdir(parents=True)
        (target / "Cargo.toml").write_text(
            '[package]\nname="composed_target"\nversion="0.1.0"'
            '\nedition="2024"\n[dependencies]\n', encoding="utf-8",
        )
        (target / "src/lib.rs").write_text('''
pub fn baseline() -> u32 { 7 }
#[cfg(test)]
mod tests { #[test] fn works() { assert_eq!(super::baseline(), 7); } }
''', encoding="utf-8")
        capsule_a = make_capsule(root, a, "twice", "double-math")
        capsule_b = make_capsule(root, b, "as_json", "json-render")
        original = (target / "src/lib.rs").read_bytes()

        verified = root / "verified"
        result = json.loads(cli(
            "verify-rust-batch", "--capsule", str(capsule_a), str(capsule_b),
            "--target", str(target), "--output", str(verified),
            "--license-reviewed", "--registry-cache", str(CARGO_CACHE),
        ))
        assert result["integrations_verified"] == 2
        assert (verified / "src/composer_double_math.rs").is_file()
        assert (verified / "src/composer_json_render.rs").is_file()
        assert (target / "src/lib.rs").read_bytes() == original
        assert not (target / "src/composer_double_math.rs").exists()
        assert (verified / "Cargo.lock").exists()
        evidence = json.loads((verified / "composer-batch-evidence.json").read_text())
        assert len(evidence["steps"]) == 2
        assert all(x["individual_evidence"]["tests_passed"] for x in evidence["steps"])
        assert all(x["individual_evidence"]["image_id"].startswith("sha256:")
                   for x in evidence["steps"])
        assert "serde_json = 1" in evidence["steps"][1]["dependencies_added"]
        provenance = json.loads((verified / "composer-source-attributions.json").read_text())
        assert {entry["capability_id"] for entry in provenance} == {"double-math", "json-render"}
        for entry in provenance:
            assert entry["license_review_asserted_by_caller"] is True
            assert entry["automated_legal_clearance"] is False
            assert len(entry["license_documents"]) == 1
            assert (verified / entry["license_documents"][0]["published_path"]).is_file()
        print("PASS multi-repo composition: 2 capsules, retained licenses, unchanged target")

        # Second source deliberately violates its own included test.
        bad = root / "source-b-bad"
        shutil.copytree(b, bad)
        source_file = bad / "src/feature.rs"
        source_file.write_text(
            source_file.read_text().replace(
                'assert_eq!(super::as_json(), "42")',
                'assert_eq!(super::as_json(), "43")',
            ), encoding="utf-8",
        )
        capsule_bad = make_capsule(root, bad, "as_json", "json-failing")
        rejected = root / "must-not-publish"
        cli("verify-rust-batch", "--capsule", str(capsule_a), str(capsule_bad),
            "--target", str(target), "--output", str(rejected),
            "--license-reviewed", "--registry-cache", str(CARGO_CACHE),
            expect_success=False)
        assert not rejected.exists()
        assert not list(root.glob(".composer-batch-*"))
        assert (target / "src/lib.rs").read_bytes() == original
        print("PASS all-or-nothing rollback: second-stage test failure rejects batch")
        verify_nested_modules(root, target)
        verify_cargo_dependency_features(root, target)


if __name__ == "__main__":
    main()
