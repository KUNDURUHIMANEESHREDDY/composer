# Verified multi-file Rust integration: pinned public-source test

Tested on a disposable Windows development workspace, 2026-11-26 (Asia/Kolkata).

## Reviewed source

- Repository: https://github.com/EbTech/rust-algorithms
- Pinned commit: `2073670c540935960849f365e1895f558fa4319d`
- Seed: `BitRevIterator` in `src/math/fft.rs`
- Included source paths: `src/math/fft.rs`, `src/math/num.rs`
- Dependency: the FFT module imports `CommonField`, `Complex`, and `PI` through `super::num`
- Root `LICENSE`: MIT, copyright Aram Ebtekar; original license notice preserved in published output
- Review scope: selected source paths and their imports; this does **not** establish a full security audit, automated licensing clearance, or source provenance attestation

## Directly observed checks

1. Selected the source with Composer's repository analyzer; created a Feature Capsule from the exact seed and its locally resolved dependency edge.
2. Verified the capsule contains both original source files and the root MIT license notice; the capsule recorded the pinned source commit.
3. Ran `composer verify-rust` against a separate, disposable, standalone Rust target in network-disabled Docker using the existing local `rust:1.99-slim` image.
4. Rust compilation and **10 tests passed, 0 failed**; no additional Cargo dependencies were required.
5. Confirmed the new namespaced Rust module tree exists, its source attribution records the pinned SHA, and the original target does not contain Composer's injected module.
6. Separately, the automated synthetic three-file nested-module test verifies `crate::` rewrites, active `serde_json` adoption, and rejection without publication when a nested source test fails.

Reproducibility: local capsule and target copies reside under `composer-realworld-validation`, outside the Composer Git repository. Test output contains `composer-build-evidence.json`, `composer-source-attributions.json`, and original `third_party_licenses` notice files. The GitHub Actions workflow exercises a deterministic, generated fixture, not this live GitHub checkout.

## What remains unproven

This is **one pinned two-file public-source case** and a generated target, not a statistical reliability measure for arbitrary multi-repository transplants. Workspaces, renames/API adaptation, conditional compilation, macros, complex Cargo features, non-Rust languages, heterogeneous projects, and general automatic repair remain unsupported or unverified. Docker verification alone is not a production-grade hostile-code sandbox.
