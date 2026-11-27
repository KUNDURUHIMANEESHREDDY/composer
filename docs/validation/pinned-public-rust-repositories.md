# Pinned public Rust repository validation — 2026-11-26

## Purpose

Replace synthetic-only demonstrations with actual source extracted from two unfamiliar public repositories. Each source is pinned to an immutable commit, inspected, bundled with license evidence, integrated into a disposable target, and verified by Cargo tests in a restricted offline Docker container. No third-party source is compiled or executed directly by the host control plane.

## Pinned sources and review

| Repository | Immutable revision | Selected file | Capability | License evidence |
| --- | --- | --- | --- | --- |
| [TheAlgorithms/Rust](https://github.com/TheAlgorithms/Rust) | 976f1cdcdab3fab85301c38b28c28e6c5c503e9a | src/math/gcd_of_n_numbers.rs | public-gcd | Root LICENSE: MIT, copyright The Algorithms |
| [EbTech/rust-algorithms](https://github.com/EbTech/rust-algorithms) | 2073670c540935960849f365e1895f558fa4319d | src/order.rs | public-merge-sort | Root LICENSE: MIT, copyright Aram Ebtekar |

The source files were inspected for obvious unsafe, filesystem, network, process-execution and inclusion operations. GCD is a self-contained arithmetic module with inline unit tests. The second capsule selects a merge-sort symbol but includes all of src/order.rs, because the current Feature Capsule extraction boundary is a whole file, not a single function. This is a limited source review and is **not** a security audit or legal clearance.

The full commit IDs, original file paths, source digests, and original MIT notices are preserved in each Feature Capsule. Published integrations now also contain composer-source-attributions.json and third_party_licenses/*.txt. When an existing repository serves as the target, its root LICENSE and common NOTICE files are retained too. The caller's --license-reviewed flag is an assertion of review; it is not automatic permission analysis.

## Actual observed failures before the verified-only adjustment

- TheAlgorithms root Cargo.toml contains dev-dependencies, optional feature dependencies and additional package dependencies. The old verifier rejected the dev-dependencies section even for this dependency-free GCD file.
- EbTech's valid Cargo.toml has no dependencies table; the old verifier rejected this before Cargo ran.
- Both selected source files contain ordinary local test-module imports (use super::*). The static import graph called these unresolved, despite the Rust compiler being able to verify them.

## Bounded changes under test

- In the compiler-verified single-file Rust path only, select direct dependency constraints that are textually referenced by the extracted file. Ignore unrelated optional, development and other upstream package manifest sections. This is *lexical selection*, not general semantic dependency analysis; Cargo must compile/test and verify the locked versions before publishing.
- Only the compiler-verified path can defer known local super:: and self:: imports and static compatibility findings to isolated Rust compilation. The unverified wire-rust command retains its stricter rejection policy.
- Require original license evidence for published verified copies, retain upstream notices and exact source provenance across multi-capsule stages, and preserve existing target license files.
- Keep offline Docker execution, bounded log/output, source and target immutability, at-most-once lockfile repair, and all-or-nothing publication.

## Live validation results

| Check | Outcome |
| --- | --- |
| TheAlgorithms GCD capsule into a disposable Rust target | Passed compilation and source unit tests inside Docker |
| EbTech order/merge-sort capsule into a disposable Rust target | Passed compilation and source unit tests inside Docker |
| Both pinned public-source capsules integrated sequentially into one disposable target | Passed both isolated stages; aggregate batch evidence says two verified stages |
| TheAlgorithms GCD integrated into a disposable copy of EbTech's **real repository** as target | Passed existing target tests and source tests in Docker |
| Source integrity and provenance | Exact pinned commits recorded, source file hashes retained, both upstream MIT notice files carried into final two-module output |
| Existing public target preservation | Original EbTech checkout unchanged; its original root LICENSE retained in integrated copy |

No test above establishes success for arbitrary repositories. The real target demonstration uses an existing open-source application, not a deployment or user project.

## Reproduction and local evidence

Local validation workspace: composer-realworld-validation, separate from the Composer Git repository. Checked-out repositories live in repos/, source capsules in capsules/, independent outputs in baseline-public-gcd/ and baseline-public-merge-sort/, combined output in public-batch-licensed/, and existing-target integration in ebtech-integrated/.

The source repository revisions were checked using git rev-parse HEAD. Composer's analyze command selected gcd from src/math/gcd_of_n_numbers.rs and merge_sort from src/order.rs. The capsule command selected those exact symbol IDs and retained their original source/manifest/license files. The verify-rust and verify-rust-batch commands used --license-reviewed and separate --output directories. No upstream checkout was modified.

Key machine-readable evidence in each verified output:
- composer-build-evidence.json: isolated test result, Cargo/lockfile status, selected Docker image ID, log excerpt and any bounded lockfile repair.
- composer-batch-evidence.json (combined output): each independently verified stage and dependencies added.
- composer-source-attributions.json: original repository URL, commit, feature IDs, source digest references and published license locations.
- third_party_licenses/: copies of upstream license evidence.

The repeatable synthetic integration/rollback test in tests/test_verified_composition_e2e.py is also extended to assert retention of individual license files and source attribution across batch stages. That deterministic test remains in CI; the public-repository study is a **separate manually reviewed benchmark**, not an automatic untrusted network fetch in CI.

## Remaining limitations and acceptance criteria

- Whole-file extraction can include unrelated functions or code; this is not true general-purpose feature adaptation.
- Lexical dependency selection cannot reliably understand compiler feature flags, macro expansion, renamed crates, build scripts, conditional compilation, or nested Cargo workspaces. Compiler failure blocks publication but tests cannot prove absence of all runtime flaws.
- Two pinned MIT repositories are a meaningful real-world smoke test, not evidence of success rates across many unrelated repositories, license categories or complex application architectures.
- Docker is restricted but not production-grade hostile-code isolation. MicroVM-grade execution, signed image provenance, strict supply-chain controls, adversarial testing, and general autonomous code repair remain outstanding.
