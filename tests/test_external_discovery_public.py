"""Discover real existing capabilities through Cargo, then verify a reused upstream.

The discovery phase does not execute source or auto-grant license approval.
Only previously reviewed third-party source is run via the Docker verifier.
"""
from __future__ import annotations

import json
import shutil
import tempfile
from pathlib import Path

from test_pinned_public_multifile import CACHE, clone_exact, run

TARGET_URL = "https://github.com/ijt/trigram.git"
TARGET_SHA = "45af254dcd7b5c8aac4feb336192204791cd1e2f"
CONTRACT = "tests/composer_discovered_contract.rs"


def cli(*args: str, require_success: bool = True):
    return run(
        "cargo", "run", "--quiet", "-p", "composer-cli", "--",
        *args, require_success=require_success, timeout=300,
    )


def discovery(root: Path, query: str, function: str, owner: str = ""):
    output = root / f"{query}-candidates.json"
    command = [
        "discover-external-rust", query, "--function", function,
        "--registry-cache", str(CACHE),
        "--limit", "4", "--output", str(output),
        "--exclude-owner", "KUNDURUHIMANEESHREDDY",
    ]
    if owner:
        command += ["--exclude-owner", owner]
    report = json.loads(cli(*command).stdout)
    assert output.is_file()
    candidates = json.loads(output.read_text())
    assert candidates == report["candidates"]
    assert candidates
    assert report["inspected_packages"] >= 1
    for c in candidates:
        assert c["upstream_repository"].startswith("https://github.com/")
        assert not c["upstream_repository"].startswith("https://github.com/KUNDURUHIMANEESHREDDY/")
        assert len(c["upstream_commit"]) == 40
        assert c["version_requirement"].count(".") >= 2
        assert c["public_symbol"] == function or c["public_symbol"].endswith("::" + function)
    return candidates, output


def verify_discovered_import(root: Path, original_target: Path, found: dict) -> None:
    target = root / "discovered-target"
    shutil.copytree(original_target, target, ignore=shutil.ignore_patterns(".git"))
    (target / "tests").mkdir(exist_ok=True)
    (target / CONTRACT).write_text(
        "use trigram::ported_jaro;\n"
        "#[test] fn independent_behavior() {\n"
        " assert_eq!(ported_jaro(\"abc\", \"abc\"), 1.0);\n"
        " assert!((ported_jaro(\"MARTHA\", \"MARHTA\") - 17.0/18.0).abs() < 1e-10);\n"
        " assert_eq!(ported_jaro(\"abc\", \"xyz\"), 0.0);\n"
        "}\n",
        encoding="utf-8",
    )
    baseline = [(target / item).read_bytes() for item in
                ["Cargo.toml", "src/lib.rs", CONTRACT]]
    # Cargo fetch populates the existing registry only. It never compiles or
    # executes the downloaded crate's build.rs on the host.
    prefetch = root / "registry-prefetch"
    shutil.copytree(original_target, prefetch, ignore=shutil.ignore_patterns(".git"))
    manifest = prefetch / "Cargo.toml"
    data = manifest.read_text()
    assert "[dependencies]" in data
    manifest.write_text(data.replace(
        "[dependencies]",
        f'[dependencies]\n{found["package"]} = "={found["version_requirement"]}"',
        1,
    ))
    run("cargo", "fetch", "--manifest-path", str(manifest),
        "--target", "x86_64-unknown-linux-gnu", timeout=180)
    candidates_path = root / "selected-discovery.json"
    candidates_path.write_text(json.dumps([found]))
    output = root / "discovered-verified"
    result = json.loads(cli(
        "recover-external-rust-crate", str(target),
        "--output", str(output),
        "--candidates-json", str(candidates_path),
        "--target-alias", "ported_jaro",
        "--contract-test", CONTRACT,
        "--registry-cache", str(CACHE),
        "--license-reviewed",
    ).stdout)
    assert result["completed"] is True, result
    assert result["tests_passed"] is True
    assert result["selected_package"] == found["package"]
    assert result["selected_version"] == found["version_requirement"]
    assert result["selected_symbol"] == found["public_symbol"]
    assert output.is_dir()
    for item, expected in zip(["Cargo.toml", "src/lib.rs", CONTRACT], baseline):
        assert (target / item).read_bytes() == expected
    evidence = json.loads((output / "composer-recovery-evidence.json").read_text())
    assert evidence["network_disabled"] is True
    assert evidence["tests_passed"] is True
    assert evidence["provenance_commit_matched_to_registry_artifact"] is False
    print("PASS auto-discovered external capability was compiled and independently tested "
          "without touching original", flush=True)


def main() -> None:
    run("docker", "image", "inspect", "rust:1.99-slim")
    with tempfile.TemporaryDirectory(prefix="composer-public-discovery-") as tmp:
        root = Path(tmp)
        target = root / "original-trigram"
        clone_exact(TARGET_URL, TARGET_SHA, target)

        found, _ = discovery(root, "strsim", "jaro")
        matching = [c for c in found if c["package"] == "strsim"
                    and "rapidfuzz/strsim-rs" in c["upstream_repository"]]
        assert matching, found
        assert matching[0]["public_symbol"] == "jaro"
        verify_discovered_import(root, target, matching[0])

        alternate, _ = discovery(root, "textdistance", "jaro")
        matching_alternate = [c for c in alternate if c["package"] == "textdistance"
                              and "life4/textdistance.rs" in c["upstream_repository"]]
        assert matching_alternate, alternate
        assert matching_alternate[0]["public_symbol"] == "str::jaro", matching_alternate

        # Negative: the same GitHub owner explicitly excluded cannot be suggested.
        excluded = cli(
            "discover-external-rust", "strsim", "--function", "jaro",
            "--registry-cache", str(CACHE),
            "--exclude-owner", "rapidfuzz",
            "--exclude-owner", "KUNDURUHIMANEESHREDDY",
            "--limit", "3", "--output", str(root / "excluded.json"),
            require_success=False,
        )
        if excluded.returncode == 0:
            blocked = json.loads(excluded.stdout)["candidates"]
            assert all("github.com/rapidfuzz/" not in c["upstream_repository"] for c in blocked)
        else:
            assert "no statically supported external function candidates" in excluded.stderr
        print("PASS GitHub-owner exclusion keeps disallowed source out", flush=True)
        print("EVIDENCE_JSON " + json.dumps({
            "discovered_primary_repo": matching[0]["upstream_repository"],
            "discovered_primary_version": matching[0]["version_requirement"],
            "discovered_alternate_repo": matching_alternate[0]["upstream_repository"],
            "discovered_alternate_symbol": matching_alternate[0]["public_symbol"],
            "source_execution_on_host": False,
            "independent_target_contract": "passed inside offline Docker",
            "owner_exclusion": "passed",
            "origin_sha_is_release_proof": False,
            "limits": "Registry search only; license review is still external; "
                      "GitHub HEAD does not cryptographically attest to published crate; "
                      "one public-target integration is not evidence of general source selection.",
        }, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
