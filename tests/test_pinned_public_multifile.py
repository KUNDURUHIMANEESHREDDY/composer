"""Pinned, reviewed Rust source/target benchmark (run manually in isolated CI).

Only trusted Composer CLI is executed on the host. All third-party Rust
compilation and test execution is confined to Composer's offline Docker sandbox.
Registry preparation downloads packages but never runs their build scripts.
This is a scoped engineering check, not a legal or security attestation.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
CACHE = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))) / "registry"
SOURCE_REPO = "https://github.com/EbTech/rust-algorithms.git"
SOURCE_SHA = "2073670c540935960849f365e1895f558fa4319d"
GEOMETRY_REPO = "https://github.com/TheAlgorithms/Rust.git"
GEOMETRY_SHA = "976f1cdcdab3fab85301c38b28c28e6c5c503e9a"
TARGET_REPOS = {
    "ebtech-target": (SOURCE_REPO, SOURCE_SHA),
    "rustc-hash": (
        "https://github.com/rust-lang/rustc-hash.git",
        "fdfe9e6ebea0f0bd3de95d5af849f5d03f4b7fbb",
    ),
    "cfg-if": (
        "https://github.com/rust-lang/cfg-if.git",
        "14432a28e180ec717933714aed7828fc7ed2fd29",
    ),
}


def run(*args: str, cwd: Path = ROOT, timeout: int = 300,
        require_success: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        list(args), cwd=cwd, text=True, capture_output=True,
        encoding="utf-8", errors="replace", timeout=timeout,
    )
    if require_success and result.returncode != 0:
        raise RuntimeError(
            f"Exit {result.returncode}: {args}\n"
            f"stdout: {result.stdout[-2500:]}\n"
            f"stderr: {result.stderr[-4500:]}"
        )
    return result


def cli(*args: str, require_success: bool = True) -> subprocess.CompletedProcess[str]:
    return run(
        "cargo", "run", "--quiet", "-p", "composer-cli", "--",
        *args, require_success=require_success
    )


def clone_exact(repo: str, sha: str, dest: Path) -> None:
    run("git", "clone", "--quiet", "--filter=blob:none", "--no-checkout",
        repo, str(dest), timeout=120)
    run("git", "-C", str(dest), "fetch", "--quiet", "--depth", "1",
        "origin", sha, timeout=120)
    run("git", "-C", str(dest), "checkout", "--quiet", "--detach", sha)
    observed = run("git", "-C", str(dest), "rev-parse", "HEAD").stdout.strip()
    assert observed == sha, f"Source revision mismatch: {observed} != {sha}"
    print(f"PINNED {repo} {sha}", flush=True)


def make_capsule(source: Path, root: Path, symbol: str,
                 file_path: str, capability: str, required: set[str],
                 expected_sha: str = SOURCE_SHA) -> Path:
    graph = json.loads(cli("analyze", str(source)).stdout)
    seeds = [row["id"] for row in graph["symbols"]
             if row["name"] == symbol
             and row["file_path"].replace("\\", "/") == file_path]
    assert len(seeds) == 1, (file_path, symbol, seeds)
    capsule = root / f"capsule-{capability}"
    cli("capsule", str(source), "--capability-id", capability,
        "--description", "Pinned reviewed public Rust dependency graph",
        "--seed-symbol", seeds[0], "--output", str(capsule))
    manifest = json.loads((capsule / "manifest.json").read_text())
    included = {value.replace("\\", "/")
                for value in manifest["included_source_files"]}
    assert required.issubset(included), f"Missing module dependency: {required - included}"
    assert manifest["source_commit_sha"] == expected_sha
    assert "LICENSE" in manifest["license_evidence_files"]
    assert manifest["license_review_required"] is True
    print(f"CAPSULE {capability} files={len(included)}: {sorted(included)}", flush=True)
    return capsule


def prepare_registry(target: Path, root: Path, name: str) -> bool:
    snapshot = root / f"registry-preparation-{name}"
    shutil.copytree(target, snapshot, ignore=shutil.ignore_patterns(".git"))
    # Cargo fetch only retrieves registry artifacts; it does not execute code.
    fetched = run("cargo", "fetch", "--manifest-path",
                  str(snapshot / "Cargo.toml"),
                  "--target", "x86_64-unknown-linux-gnu",
                  timeout=150, require_success=False)
    if fetched.returncode:
        print(
            f"CACHE_BLOCKED {name}: {fetched.stderr[-700:]}",
            flush=True
        )
        return False
    return True


def verify_feature(root: Path, capsule: Path, target: Path, name: str,
                   capability: str, api_alias: str | None = None,
                   api_contract: str | None = None,
                   expected_sha: str = SOURCE_SHA) -> bool:
    staged_target = root / f"target-input-{capability}-{name}"
    shutil.copytree(target, staged_target, ignore=shutil.ignore_patterns(".git"))
    original_lib = (staged_target / "src/lib.rs").read_bytes()
    if api_contract is not None:
        (staged_target / "tests").mkdir(exist_ok=True)
        (staged_target / "tests/imported_api.rs").write_text(
            api_contract, encoding="utf-8"
        )
    output = root / f"output-{capability}-{name}"
    command = [
        "verify-rust", str(capsule), str(staged_target),
        "--output", str(output), "--license-reviewed",
        "--registry-cache", str(CACHE),
    ]
    if api_alias is not None:
        command += ["--api-alias", api_alias]
    result = cli(*command, require_success=False)
    assert (staged_target / "src/lib.rs").read_bytes() == original_lib
    assert not (staged_target / "src" / f"composer_{capability.replace('-', '_')}").exists()
    if result.returncode:
        assert not output.exists(), "Verifier published output after failure"
        print(
            f"REJECTED {capability} -> {name}: {result.stderr[-1400:]}",
            flush=True
        )
        return False
    evidence = json.loads((output / "composer-build-evidence.json").read_text())
    attribution = json.loads((output / "composer-source-attributions.json").read_text())
    assert evidence["tests_passed"] is True
    assert evidence["network_disabled"] is True
    assert attribution[0]["source_commit_sha"] == expected_sha
    assert (output / attribution[0]["published_module"]).is_file(), (
        "Published provenance path must match the actual module tree"
    )
    assert attribution[0]["license_documents"]
    for notice in attribution[0]["license_documents"]:
        assert (output / notice["published_path"]).is_file()
    assert evidence["verified_api_aliases"] == ([api_alias] if api_alias else [])
    assert (output / "src" / f"composer_{capability.replace('-', '_')}" / "mod.rs").is_file()
    expected_target_notices = (
        ("LICENSE",) if name == "ebtech-target"
        else ("LICENSE-MIT", "LICENSE-APACHE")
    )
    for license_name in expected_target_notices:
        assert (output / license_name).is_file(), f"Target license lost: {license_name}"
    print(
        f"PASS {capability} -> {name}: "
        + " / ".join(x.strip() for x in evidence["log_tail"].splitlines()
                     if "test result: ok" in x),
        flush=True
    )
    return True


def main() -> None:
    run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-pinned-public-") as directory:
        root = Path(directory)
        source = root / "ebtech-source"
        clone_exact(SOURCE_REPO, SOURCE_SHA, source)
        assert "MIT License" in (source / "LICENSE").read_text()
        targets: dict[str, Path] = {}
        for name, (url, sha) in TARGET_REPOS.items():
            target = root / f"target-{name}"
            clone_exact(url, sha, target)
            expected_notices = (
                ("LICENSE",) if name == "ebtech-target"
                else ("LICENSE-MIT", "LICENSE-APACHE")
            )
            for notice in expected_notices:
                assert (target / notice).is_file(), (name, notice)
            targets[name] = target

        graph = make_capsule(
            source, root, "Graph", "src/graph/mod.rs", "public-graph",
            {"src/graph/mod.rs", "src/graph/flow.rs",
             "src/graph/connectivity.rs", "src/graph/util.rs"},
        )
        range_query = make_capsule(
            source, root, "StaticArq", "src/range_query/static_arq.rs",
            "public-range",
            {"src/range_query/mod.rs", "src/range_query/static_arq.rs",
             "src/range_query/specs.rs", "src/range_query/dynamic_arq.rs",
             "src/range_query/sqrt_decomp.rs"},
        )
        # Independent public-to-public positive acceptance: geometry module
        # from TheAlgorithms into the different EbTech existing Cargo project.
        geometry_source = root / "thealgorithms-source"
        clone_exact(GEOMETRY_REPO, GEOMETRY_SHA, geometry_source)
        assert "MIT License" in (geometry_source / "LICENSE").read_text()
        geometry = make_capsule(
            geometry_source, root, "closest_points",
            "src/geometry/closest_points.rs", "public-geometry",
            {"src/geometry/mod.rs", "src/geometry/point.rs",
             "src/geometry/closest_points.rs"},
            expected_sha=GEOMETRY_SHA,
        )
        assert CACHE.is_dir(), f"No Cargo registry available at {CACHE}"
        geometry_contract = (
            "use contest_algorithms::ImportedClosestPoints;\n"
            "use contest_algorithms::composer_public_geometry::geometry::Point;\n"
            "#[test] fn adapted_geometry_api_works() { "
            "let points = [Point::new(0.0, 0.0), Point::new(1.0, 0.0)]; "
            "assert!(ImportedClosestPoints(&points).is_some()); }\n"
        )
        assert verify_feature(
            root, geometry, targets["ebtech-target"], "ebtech-target",
            "public-geometry",
            "geometry::closest_points=ImportedClosestPoints",
            geometry_contract,
            expected_sha=GEOMETRY_SHA,
        ), "Cross-repository geometry integration failed"

        # Separate negative compatibility probes. These targets declare
        # no_std, whereas the imported code assumes a std prelude.
        # A rejection must leave the target and proposed output untouched.
        if prepare_registry(targets["rustc-hash"], root, "rustc-hash"):
            assert not verify_feature(
                root, graph, targets["rustc-hash"], "rustc-hash", "public-graph",
                "graph::Graph=ImportedGraph",
            ), "Unexpected std-dependent module acceptance in no_std target"
            assert not verify_feature(
                root, range_query, targets["rustc-hash"], "rustc-hash", "public-range"
            ), "Unexpected std-dependent range acceptance in no_std target"
        else:
            print("BLOCKED rustc-hash: offline registry prerequisites", flush=True)
        if prepare_registry(targets["cfg-if"], root, "cfg-if"):
            assert not verify_feature(
                root, graph, targets["cfg-if"], "cfg-if", "public-graph"
            ), "Unexpected std-dependent graph acceptance in no_std target"
        else:
            print("BLOCKED cfg-if: offline registry prerequisites", flush=True)
        print("PASS pinned cross-repository geometry API integration; "
              "no_std compatibility probes recorded", flush=True)


if __name__ == "__main__":
    main()
