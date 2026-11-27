# Pinned multi-file Rust integration and API adaptation benchmark

Observed in GitHub Actions on 2026-11-27 UTC. Every source and target commit below is pinned to an immutable revision. Test runner does not modify original Git checkout.

Successful initial benchmark: https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/45929355487

Reproduction: tests/test_pinned_public_multifile.py and .github/workflows/pinned-public-multifile.yml

## Reviewed original repositories

| Role | Repository | Pinned revision | License evidence |
| --- | --- | --- | --- |
| Source A | https://github.com/TheAlgorithms/Rust | 976f1cdcdab3fab85301c38b28c28e6c5c503e9a | Root LICENSE: MIT |
| Source B / positive target | https://github.com/EbTech/rust-algorithms | 2073670c540935960849f365e1895f558fa4319d | Root LICENSE: MIT |
| Negative target 1 | https://github.com/rust-lang/rustc-hash | fdfe9e6ebea0f0bd3de95d5af849f5d03f4b7fbb | LICENSE-MIT and LICENSE-APACHE |
| Negative target 2 | https://github.com/rust-lang/cfg-if | 14432a28e180ec717933714aed7828fc7ed2fd29 | LICENSE-MIT and LICENSE-APACHE |

The eight included Source A geometry files were inspected for obvious unsafe, filesystem/network/process execution, and include operations. This limited review does NOT establish source safety or legal clearance. Original license files were inspected and preserved.

## Verified result matrix

| Source capsule | Existing target | Result | Evidence |
| --- | --- | --- | --- |
| TheAlgorithms geometry: 8 Rust files (closest_points, graham_scan, jarvis_scan, mod, point, polygon_points, ramer_douglas_peucker, segment) | EbTech contest-algorithms library | PASS | 102 library tests passed; two separate test groups passed 1 test each; zero failures; externally compiled API contract exercised ImportedClosestPoints |
| EbTech graph: 4 Rust files | rustc-hash (no_std) | REJECTED | Rust compiler found missing std prelude types such as Vec; no output was published |
| EbTech range queries: 5 Rust files | rustc-hash (no_std) | REJECTED | Rust compiler found missing std prelude types; no output was published |
| EbTech graph: 4 Rust files | cfg-if (std-unavailable target) | REJECTED | Rust compiler found missing std prelude types; no output was published |

A rejected integration is a compatibility failure, not a successfully adapted feature. Current Composer does NOT bridge std-dependent features into no_std crates.

## API adaptation proof

The successful target integration test imports contest_algorithms::ImportedClosestPoints and calls it using two points. The explicit verification mapping, --api-alias geometry::closest_points=ImportedClosestPoints, re-exports the source function at the target crate root. Original upstream source is not manually edited. Compiler-verified namespacing relocates crate::geometry references. Composer stages source, selected Cargo constraints, test contract, and license evidence in a new target copy and publishes only after offline Docker build/tests pass.

Published evidence includes composer-build-evidence.json, composer-source-attributions.json and original notices in third_party_licenses/. The provenance path was corrected for multifile capsules to identify src/composer_<capability>/mod.rs. Target checkout remains unchanged; failed outputs are absent.

## Reproducibility and limitations

The benchmark clones pinned public revisions, fetches registry artifacts on disposable snapshots without compiling third-party code on the host, and executes Rust tests in Composer's restricted network-disabled Docker sandbox. Docker is NOT a production-grade hostile-code VM.

This is one passing eight-file transplant into one unrelated existing target plus three incompatibility probes, not a statistically significant reliability study. Composer does not yet automatically infer API semantics or adapt function signatures, support arbitrary workspace/Cargo features, bridge no_std and std, or generalize to non-Rust languages. The caller-supplied --license-reviewed is an explicit human review assertion, not automated licensing permission.
