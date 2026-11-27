# Reviewed Rust API integration: pinned repeated public evidence

Measured on GitHub Actions, 2026-11-27. This is contract-directed execution, not autonomous semantic migration.

- Successful 6-of-6 benchmark (implementation revision 3b2e13dc): https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/45938463839
- Repeatability harness: tests/test_reviewed_api_repeated_public.py
- Expanded acceptance (commit f7abe72): 6/6 hosted Docker transfers passed with 136 independently computed totient and 142 cube test vectors per repeat, generated using upstream Hypothesis 6.170.17: https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/46005058231
- Limits: vectors were bounded and reproducible; two curated source functions and targets are not an estimate of general semantic compatibility.
- Source execution confined to Composer's existing restricted network-disabled Docker verifier (rust:1.99-slim). Cargo registry artifacts are prefetched without running third-party build scripts on the host.

## Pinned repositories

| Role | Project | SHA | Original license evidence |
| --- | --- | --- | --- |
| Source | TheAlgorithms/Rust | 976f1cdcdab3fab85301c38b28c28e6c5c503e9a | LICENSE: MIT |
| Target A | EbTech/rust-algorithms | 2073670c540935960849f365e1895f558fa4319d | LICENSE: MIT |
| Target B | rust-lang/rustc-hash | fdfe9e6ebea0f0bd3de95d5af849f5d03f4b7fbb | LICENSE-MIT, LICENSE-APACHE |

Every execution used a fresh disposable copy of the original existing target repository, with a manually supplied unfinished public API stub and independently authored behavioral contract tests. The CLI re-discovered and confirmed the exact rule, safely replaced only the stub body in a new output copy, and ran the full Cargo test suite in restricted offline Docker before publishing.

## Six complete execution attempts

| Scenario | Rep 1 discovery / verification | Rep 2 | Rep 3 | Verified |
| --- | --- | --- | --- | --- |
| euler_totient(u64) to EbTech adapted_totient(u32)->u128 | 95.5 ms / 2.91 s | 94.6 ms / 2.92 s | 93.5 ms / 2.93 s | 3/3 |
| perfect_cube_binary_search(i64) to rustc-hash perfect_cube_binary_search_checked(i32)->bool | 109.4 ms / 1.19 s | 101.7 ms / 1.19 s | 101.1 ms / 1.18 s | 3/3 |

The result includes retained license/provenance evidence, passing network-disabled Cargo tests, and byte-for-byte unchanged source and target checkouts. Reported timings are from a warmed GitHub Linux runner, excluding clone/cache setup; they are not general hardware performance measurements.

## Rejections actually tested

- An intentionally wrong independent test expected adapted_totient(9_u32) == 999_u128: Cargo failed and nothing was published
- Stale signature, missing test file, path traversal and mixed manual/reviewed mode rejected without output
- Original non-placeholder functions are not rewritten
- Cargo package autotests=false is disallowed (contract test might never execute)
- Original capsule source hash and dependency/lockfile checks preserved

## Meaningful failure boundaries

A proposed EbTech fast_gcd capsule was refused for unresolved import/reference closure. A different source module was refused in glob due to the target's strict missing-documentation lint. Another candidate's unit tests used unavailable println! in a no_std context. We kept those limits rather than weakening source or target validation. The final accepted second case is self-contained and core-compatible.

## Open risks

- Only two curated public source functions, two target libraries, three runs per scenario. No general success-rate estimate.
- User-defined target API requirements and independent behavioral assertions, not automatic intent detection or synthetic self-certification.
- Cargo workspaces, arbitrary dependency graphs, generic/trait/lifetime APIs and cross-language integrations remain unsupported.
- Restricted Docker does not provide production-grade hostile-code isolation.
- Native Windows Cargo formatting, strict Clippy and 68/68 workspace tests passed on commit b5c5f0e. Live Windows Docker acceptance was not possible because its engine was unavailable; hosted Linux Docker acceptance passed at f7abe72.
