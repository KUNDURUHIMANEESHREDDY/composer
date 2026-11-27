"""Pinned, unfamiliar public-source Rust API discovery benchmark.

No third-party code is executed. Git checkouts are pinned, analyzed statically,
and copied only into disposable targets containing explicit API placeholders.
Recommendations are *not* integrations; independent compiler tests remain mandatory.
"""
from __future__ import annotations

import json
from pathlib import Path
import shutil
import tempfile

from test_pinned_public_multifile import (
    GEOMETRY_REPO, GEOMETRY_SHA, SOURCE_REPO, SOURCE_SHA,
    TARGET_REPOS, cli, clone_exact, make_capsule,
)


def recommend(capsule: Path, target: Path, function_name: str) -> dict:
    before = (target / "src/lib.rs").read_bytes()
    result = json.loads(cli("suggest-rust-api", str(capsule), str(target)).stdout)
    assert (target / "src/lib.rs").read_bytes() == before, "Discovery mutated target"
    assert result["schema_version"] == 1
    assert result["warning"].startswith("Read-only")
    matches = [
        item for item in result["requirements"]
        if item["target_function"] == function_name
    ]
    assert len(matches) == 1, (function_name, result["requirements"])
    return matches[0]


def target_with_stubs(src: Path, dest: Path, declarations: str) -> Path:
    shutil.copytree(src, dest, ignore=shutil.ignore_patterns(".git"))
    path = dest / "src/lib.rs"
    with path.open("a", encoding="utf-8") as f:
        f.write("\n// Unimplemented public target contract for static analysis only\n")
        f.write(declarations)
    return dest


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="composer-unknown-api-") as directory:
        root = Path(directory)
        the_algorithms = root / "the-algorithms"
        ebtech = root / "ebtech"
        rustc_hash = root / "rustc-hash"
        clone_exact(GEOMETRY_REPO, GEOMETRY_SHA, the_algorithms)
        clone_exact(SOURCE_REPO, SOURCE_SHA, ebtech)
        hash_url, hash_sha = TARGET_REPOS["rustc-hash"]
        clone_exact(hash_url, hash_sha, rustc_hash)

        # Source repository A -> unrelated target repository B:
        # u32->u64 input and u64->u128 result are fixed-width widening.
        totient = make_capsule(
            the_algorithms, root, "euler_totient",
            "src/number_theory/euler_totient.rs", "auto-totient",
            {"src/number_theory/euler_totient.rs"},
            expected_sha=GEOMETRY_SHA,
        )
        target_b = target_with_stubs(
            ebtech, root / "target-b",
            "pub fn adapted_totient(n: u32) -> u128 { todo!() }\n"
            "pub fn unrelated_password(n: u32) -> u128 { todo!() }\n"
            "pub fn already_done(n: u32) -> u128 { n.into() }\n"
        )
        good = recommend(totient, target_b, "adapted_totient")
        assert good["best_suggestion"]["suggested_cli_rule"] == (
            "euler_totient=adapted_totient(u32)->u128"
        ), good
        assert good["status"].startswith("candidate_requires_independent")
        unrelated = recommend(totient, target_b, "unrelated_password")
        assert unrelated["best_suggestion"] is None
        assert all(row["target_function"] != "already_done"
                   for row in json.loads(cli(
                       "suggest-rust-api", str(totient), str(target_b)
                   ).stdout)["requirements"])
        print("PASS repo A->B: deterministic name and widening suggestion; "
              "unrelated and implemented methods ignored", flush=True)

        # Source repository B -> existing target A, independent of above names:
        # signed i32->i64 input widening; i64 return stays i64.
        fast_gcd = make_capsule(
            ebtech, root, "fast_gcd",
            "src/math/num.rs", "auto-fast-gcd",
            {"src/math/num.rs"},
        )
        target_a = target_with_stubs(
            the_algorithms, root / "target-a",
            "pub fn fast_gcd_checked(a: i32, b: i32) -> i64 { unimplemented!() }\n"
            "pub fn fast_gcd_narrow(a: i64, b: i64) -> i32 { todo!() }\n"
        )
        matched = recommend(fast_gcd, target_a, "fast_gcd_checked")
        assert matched["best_suggestion"]["suggested_cli_rule"] == (
            "fast_gcd=fast_gcd_checked(i32,i32)->i64"
        ), matched
        incompatible = recommend(fast_gcd, target_a, "fast_gcd_narrow")
        assert incompatible["best_suggestion"] is None, incompatible
        print("PASS repo B->A: signed widening suggestion; narrowing rejected", flush=True)

        # Third unfamiliar repository has a no_std type-oriented library API,
        # not a scalar top-level function suitable for this bounded adapter.
        hash_capsule = make_capsule(
            rustc_hash, root, "FxHasher", "src/lib.rs", "auto-hasher",
            {"src/lib.rs"}, expected_sha=hash_sha,
            license_file="LICENSE-MIT",
        )
        negative = recommend(hash_capsule, target_b, "adapted_totient")
        assert negative["best_suggestion"] is None, negative
        print("PASS third public source: unsupported/no matching scalar API "
              "does not invent an adapter", flush=True)
        # Two previously unbenchmarked Rust projects serve as negative
        # controls: exposed APIs return Result/opaque types, not fixed-width
        # scalar signatures. Discovery must not fabricate matches.
        unfamiliar = [
            (
                "glob",
                "https://github.com/rust-lang/glob.git",
                "057cef2824f5a96f738e52d7d8ca3e024381bcc6",
                "glob", "auto-glob",
            ),
            (
                "demangle",
                "https://github.com/rust-lang/rustc-demangle.git",
                "4023665a9bafb7cd9808033fd76fa50f83b9542c",
                "Demangle", "auto-demangle",
            ),
        ]
        for name, url, sha, seed, capability in unfamiliar:
            checkout = root / f"unfamiliar-{name}"
            clone_exact(url, sha, checkout)
            assert (checkout / "LICENSE-MIT").is_file()
            feature = make_capsule(
                checkout, root, seed, "src/lib.rs", capability,
                {"src/lib.rs"}, expected_sha=sha, license_file="LICENSE-MIT",
            )
            found = recommend(feature, target_b, "adapted_totient")
            assert found["best_suggestion"] is None, (name, found)
            print(f"PASS unseen public source {name}: incompatible opaque API "
                  "rejected without fabrication", flush=True)
        print("PASS 5 distinct pinned public sources, 2 existing project targets; "
              "discovery read-only, no unreviewed execution", flush=True)


if __name__ == "__main__":
    main()
