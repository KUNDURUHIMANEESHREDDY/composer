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
TARGET_REPOS = {
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
                 file_path: str, capability: str, required: set[str]) -> Path:
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
    assert manifest["source_commit_sha"] == SOURCE_SHA
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
                   api_contract: str | None = None) -> bool:
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
    assert attribution[0]["source_commit_sha"] == SOURCE_SHA
    assert attribution[0]["license_documents"]
    for notice in attribution[0]["license_documents"]:
        assert (output / notice["published_path"]).is_file()
    assert evidence["verified_api_aliases"] == ([api_alias] if api_alias else [])
    assert (output / "src" / f"composer_{capability.replace('-', '_')}" / "mod.rs").is_file()
    for license_name in ("LICENSE-MIT", "LICENSE-APACHE"):
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
            for notice in ("LICENSE-MIT", "LICENSE-APACHE"):
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
        assert CACHE.is_dir(), f"No Cargo registry available at {CACHE}"
        # Positive acceptance: real existing target and a compiled API contract.
        assert prepare_registry(targets["rustc-hash"], root, "rustc-hash")
        contract = (
            "use rustc_hash::ImportedGraph;\n"
            "#[test] fn graph_is_exported() { "
            "assert!(std::mem::size_of::<ImportedGraph>() > 0); }\n"
        )
        first = verify_feature(
            root, graph, targets["rustc-hash"], "rustc-hash",
            "public-graph", "graph::Graph=ImportedGraph", contract
        )
        second = verify_feature(
            root, range_query, targets["rustc-hash"], "rustc-hash",
            "public-range"
        )
        assert first and second, "Reviewed public-source integration did not pass"
        # Compatibility probe against an independently pinned second target.
        if prepare_registry(targets["cfg-if"], root, "cfg-if"):
            verify_feature(root, graph, targets["cfg-if"], "cfg-if", "public-graph")
        else:
            print("BLOCKED cfg-if target: offline registry prerequisites", flush=True)
        print("PASS required existing-target and multi-file API adaptation checks",
              flush=True)


if __name__ == "__main__":
    main()
