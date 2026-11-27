# Pinned multi-file Rust integration and API adaptation benchmark

Observed in GitHub Actions on 2026-11-27 UTC. Every source and target commit below is pinned to an immutable revision. Test runner does not modify original Git checkout.

Successful initial multi-file benchmark: https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/45929355487

Successful extended signature benchmark: https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/45931010604

Reproduction: tests/test_pinned_public_multifile.py and .github/workflows/pinned-public-multifile.yml

## Reviewed original repositories

| Role | Repository | Pinned revision | License evidence |
| --- | --- | --- | --- |
| Source A | https://github.com/TheAlgorithms/Rust | 976f1cdcdab3fab85301c38b28c28e6c5c503e9a | Root LICENSE: MIT |
| Source B / positive target | https://github.com/EbTech/rust-algorithms | 2073670c540935960849f365e1895f558fa4319d | Root LICENSE: MIT |
| Negative target 1 | https://github.com/rust-lang/rustc-hash | fdfe9e6ebea0f0bd3de95d5af849f5d03f4b7fbb | LICENSE-MIT and LICENSE-APACHE |
| Negative target 2 | https://github.com/rust-lang/cfg-if | 14432a28e180ec717933714aed7828fc7ed2fd29 | LICENSE-MIT and LICENSE-APACHE |

The eight included Source A geometry files and the standalone euler_totient.rs function were inspected for obvious unsafe, filesystem/network/process execution, and include operations. This limited review does NOT establish source safety or legal clearance. Original license files were inspected and preserved.

## Verified result matrix

| Source capsule | Existing target | Result | Evidence |
| --- | --- | --- | --- |
| TheAlgorithms geometry: 8 Rust files (closest_points, graham_scan, jarvis_scan, mod, point, polygon_points, ramer_douglas_peucker, segment) | EbTech contest-algorithms library | PASS | 102 library tests passed; two separate test groups passed 1 test each; zero failures; externally compiled API contract exercised ImportedClosestPoints |
| TheAlgorithms euler_totient.rs: standalone public Rust function (u64 -> u64) | Same pinned EbTech existing library, independently copied | PASS | Compiler-inferred wrapper exposes adapted_totient(u32) -> u128. A target integration test checked adapted_totient(9_u32) == 6_u128. Docker output: 83 library tests plus two separate one-test groups passed; zero failures |
| EbTech graph: 4 Rust files | rustc-hash (no_std) | REJECTED | Rust compiler found missing std prelude types such as Vec; no output was published |
| EbTech range queries: 5 Rust files | rustc-hash (no_std) | REJECTED | Rust compiler found missing std prelude types; no output was published |
| EbTech graph: 4 Rust files | cfg-if (std-unavailable target) | REJECTED | Rust compiler found missing std prelude types; no output was published |

A rejected integration is a compatibility failure, not a successfully adapted feature. Current Composer does NOT bridge std-dependent features into no_std crates.

## API adaptation proof

The successful eight-file integration test imports contest_algorithms::ImportedClosestPoints and calls it using two points. The explicit verification mapping, --api-alias geometry::closest_points=ImportedClosestPoints, re-exports the source function at the target crate root. Original upstream source is not manually edited. Compiler-verified namespacing relocates crate::geometry references. Composer stages source, selected Cargo constraints, test contract, and license evidence in a new target copy and publishes only after offline Docker build/tests pass.

The separate numeric-function test used --api-signature "euler_totient=adapted_totient(u32)->u128", generating a Rust wrapper where u32 input becomes the original u64 parameter and the u64 result becomes u128 using compiler-inferred Into conversions. It required independent observed function behavior, not just a passing build. It does not infer desired user signatures without an explicit target contract.

Published evidence includes composer-build-evidence.json, composer-source-attributions.json and original notices in third_party_licenses/. The provenance path was corrected for multifile capsules to identify src/composer_<capability>/mod.rs. Target checkout remains unchanged; failed outputs are absent.

## Reproducibility and limitations

The benchmark clones pinned public revisions, fetches registry artifacts on disposable snapshots without compiling third-party code on the host, and executes Rust tests in Composer's restricted network-disabled Docker sandbox. Docker is NOT a production-grade hostile-code VM.

This is two positive integrations (one eight-file source feature, one single-file numeric signature adapter) into an unrelated existing target, plus three incompatibility probes; it is not a statistically significant reliability study. Composer supports only explicit, bounded compiler-inferred Into API wrappers, not arbitrary automatic signature discovery or semantic translation. Cargo feature propagation is limited to used direct registry dependencies with literal version/features tables; arbitrary workspace/Cargo configurations, no_std to std bridging and non-Rust languages remain unsupported. The caller-supplied --license-reviewed is an explicit human review assertion, not automated licensing permission.
