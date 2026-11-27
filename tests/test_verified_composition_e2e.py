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
        verified_multi_file_case(root)



def verified_multi_file_case(root: Path) -> None:
    """Real Rust graph: lib -> math.rs -> math/util.rs, plus crate path rewrite."""
    source = root / "nested_source"
    (source / "src/math").mkdir(parents=True)
    (source / "Cargo.toml").write_text(
        '[package]\nname="nested_source"\nversion="0.1.0"\nedition="2024"'
        '\nlicense="MIT"\n[dependencies]\nserde_json="1"\n', encoding="utf-8",
    )
    (source / "LICENSE").write_text(
        "MIT License - controlled original test fixture\n", encoding="utf-8"
    )
    (source / "src/lib.rs").write_text(
        "pub mod math;\n"
        "pub fn calculate(value: u32) -> u32 { crate::math::double(value) }\n"
        "#[cfg(test)]\nmod tests {\n"
        "  #[test] fn root_api() { assert_eq!(super::calculate(21), 42); }\n"
        "}\n", encoding="utf-8",
    )
    (source / "src/math.rs").write_text(
        "pub mod util;\n"
        "pub fn double(value: u32) -> u32 { self::util::scale(value) }\n"
        "pub fn as_json(value: u32) -> String {\n"
        "  serde_json::to_string(&double(value)).unwrap()\n}\n"
        "#[cfg(test)]\nmod tests {\n"
        "  #[test] fn intermediate_api() { assert_eq!(super::double(9), 18); }\n"
        "}\n", encoding="utf-8",
    )
    (source / "src/math/util.rs").write_text(
        "pub fn scale(value: u32) -> u32 { value * 2 }\n"
        "#[cfg(test)]\nmod tests {\n"
        "  #[test] fn nested_leaf() { assert_eq!(super::scale(4), 8); }\n"
        "}\n", encoding="utf-8",
    )
    target = root / "nested_target"
    (target / "src").mkdir(parents=True)
    (target / "Cargo.toml").write_text(
        '[package]\nname="nested_target"\nversion="0.1.0"'
        '\nedition="2024"\n[dependencies]\n', encoding="utf-8",
    )
    (target / "src/lib.rs").write_text(
        'pub fn baseline() -> u32 { 7 }\n'
        '#[cfg(test)] mod tests {\n'
        ' #[test] fn untouched() { assert_eq!(super::baseline(), 7); }\n'
        '}\n', encoding="utf-8",
    )
    original = (target / "src/lib.rs").read_bytes()
    capsule = make_capsule(root, source, "calculate", "nested-calculator")
    manifest = json.loads((capsule / "manifest.json").read_text())
    included = {str(path).replace("\\", "/")
                for path in manifest["included_source_files"]}
    assert included == {"src/lib.rs", "src/math.rs", "src/math/util.rs"}, included
    output = root / "nested_verified"
    result = json.loads(cli(
        "verify-rust", str(capsule), str(target), "--output", str(output),
        "--license-reviewed", "--registry-cache", str(CARGO_CACHE),
    ))
    namespace = output / "src/composer_nested_calculator"
    assert (namespace / "mod.rs").is_file()
    assert (namespace / "math.rs").is_file()
    assert (namespace / "math/util.rs").is_file()
    assert "crate::composer_nested_calculator::math::double(value)" in (
        namespace / "mod.rs"
    ).read_text()
    assert "serde_json = 1" in result["new_dependencies"]
    assert result["isolated_tests_passed"] is True
    assert (target / "src/lib.rs").read_bytes() == original
    assert not (target / "src/composer_nested_calculator").exists()
    evidence = json.loads((output / "composer-build-evidence.json").read_text())
    assert "test result: ok." in evidence["log_tail"]
    attribution = json.loads((output / "composer-source-attributions.json").read_text())
    assert len(attribution[0]["source_files"]) == 3
    print("PASS verified 3-file Rust module tree, import rewrite, offline deps, tests")



if __name__ == "__main__":
    main()
