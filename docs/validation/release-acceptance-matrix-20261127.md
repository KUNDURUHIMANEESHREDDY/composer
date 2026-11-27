# Composer release acceptance matrix — 27 November 2026

This report distinguishes **observed test results** from expectations. A green
workflow proves only the assertions its jobs actually perform. No claim of
complete supply-chain security or arbitrary cross-repository composition is
authorized by this document.

| Area | Current evidence | Release decision |
| --- | --- | --- |
| Integrated main candidate | PR #34 merge commit `1d3d323`: 15/15 workflows passed, including Linux gVisor, locked native Windows Rust build, RustSec, NSIS install/uninstall, and GitHub build attestations | Integrated baseline verified; follow-on commits require independent repeat |
| Previously unseen static analysis | `875c09c`: held-out 30/30 Git-pinned Rust repos, identical two-run analysis, untouched originals, 3 shards; [Actions evidence](https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/45992440098) | STATIC inspection pass only |
| Static scope size | 3,062 files parsed; 56,129 symbols reported. Individual case elapsed includes checkout + two analyses: p50 0.604s, p95 2.060s, max 2.518s on GitHub-hosted Linux; not an installed-Windows benchmark | Supporting diagnostics, not a production performance SLA |
| Held-out functional integration | `dtolnay/anyhow` passed 3/3 full-scope isolated runs, plus wrong-function rejection; [evidence](https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/45992883550). `rust-num/num-traits` separately passed 3/3 full-scope runs and wrong-function rejection; [evidence](https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/45993697881). Two other trials exposed a benchmark-compatibility blocker and an evidence assertion bug; see below. | **2/30 different full-scope functional targets verified**; 28/30 remain unproven |
| Broad behavioral compatibility | Only a few curated external capability/target tests have passed. Zero behavioral integrations were attempted in the 30-repo static workflow | **Not proven**; pre-register 30 functional contracts or honest unsupported cases, count all failures, repeat three times |
| Host Git execution trust boundary | Effective checkout config scanned for clean/smudge/process helpers and fsmonitor disabled; newly added cross-platform tests; source-acquisition host Git now excludes inherited config overrides and templates | Requires exact-head tests and independent hostile-input review |
| Dependency and pipeline supply chain | RustSec advisory scan, pinned GitHub Actions, Syft SPDX SBOM, Cargo Git commit/tree/blob verification, GitHub Sigstore-backed build attestation | Useful controls; full review of upstream signer identities, SBOM completeness, runtime and build infrastructure still open |
| Windows installer | GitHub hosted Windows NSIS install, CLI smoke, uninstall and unrelated-file-preservation acceptance passed | Not Authenticode-signed; actual distribution/update/rollback and independent review open |
| Linux runsc | Existing Linux gVisor CI run passed | Not a proof of complete sandbox escape resistance |
| Windows Docker Desktop runsc | Connected host: Docker CLI available, Linux daemon absent, `runsc` not verified, ~1 GiB free RAM at check | **Blocked**; do not kill user programs or silently downgrade runtime |
| Repository release controls | The connected machine's authenticated `gh api` returned `404 Branch not protected` for `main`; rulesets count is `0`. No protections were changed in this audit. | **Blocked**: release owner must adopt required checks/reviews and force-push/deletion restrictions before approval |
| External security approval | No independent reviewer sign-off or penetration/security acceptance report | **Blocked** |
| Production readiness | Incomplete functional evidence, independent security approval, local Windows runsc, actual signed release/update/rollback | **Blocked: no production designation** |

Open audit: [issue #35](https://github.com/KUNDURUHIMANEESHREDDY/composer/issues/35).
The draft PR remains separate from `main` and must not be merged simply because
a smaller subset of unrelated CI workflows passes.

## Explicit new-target failure classifications

The first extra-target matrix run [45993697881](https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/45993697881) reported:

- `rust-num/num-traits`: **3/3 full-target successes**, semantic negative control rejected; this is a second distinct target, not a second source capability.
- `unicode-rs/unicode-segmentation`: Composer returned successful full-scope test evidence in its first attempt, but the Python acceptance harness incorrectly insisted that `test result: ok` be present in a **truncated log tail**. This is a test-oracle failure, not a demonstrated Cargo failure. Follow-up requires the real three repetitions and negative control; no completed success is claimed yet.
- `bitflags/bitflags`: default `cargo test --locked --offline --all-targets` genuinely failed because `benches/parse.rs` contains nightly-only `#![feature(test)]`. This counts as **no full-target support** on the stable verifier. Follow-up must first verify refusal without publication, then exercise Composer's existing explicit `--contract-focused` mode and label `library_and_integration_tests` as **weaker coverage**. Do not mark an excluded benchmark as verified.

The follow-up regression retains all independent contract tests, all exact Git provenance checks, offline Docker isolation, and the wrong-but-type-compatible function rejection. No silent fallback from the default all-targets scope is permitted.

## Exact-container-image supply chain hardening (pending exact-head CI)

The existing CI had 17 floating `docker pull rust:1.99-slim` occurrences.
Using upstream Docker Buildx `imagetools inspect` against the official Docker
Hub `library/rust:1.99-slim` OCI image index on 27 November 2026, the
manifest digest was observed as
`sha256:f6c4a35d21dbb3c89d7a8d056c193e94fc09037f2b58dd858b09c39a47dd02d7`.
Every CI prefetch now reuses Docker to pull that immutable digest in
`scripts/prefetch-pinned-rust-verifier.sh` and only then aliases it to
`rust:1.99-slim` for Composer's existing strict `--pull=never` offline
verifier. The script refuses a missing/mismatched image ID. A CI regression
rejects floating verifier pulls. This is *image content immutability*, not
independent audit of official Docker/Rust image maintainers or the Docker daemon.
Installed Windows runsc acceptance remains blocked. This change must pass
GitHub Linux CI on the updated SHA before being counted as verified.

## Failure-directed resource admission for large third-party test suites

The initial `bitflags` scoped run failed with linker bus errors and
`No space left on device (os error 28)` in its 41-case `trybuild`
compile-test suite; the Docker verifier's `/work` tmpfs default was 1 GiB.
This is **a verified failure**, not a semantic test pass. Composer now permits
`COMPOSER_VERIFIER_MEMORY_MB=2048` or `4096` as an **explicit per-run
resource admission**; the unchanged default remains `1024`. Unsupported,
zero, negative or unbounded values fail closed. Both the Docker container
memory limit and ephemeral `/work` tmpfs remain capped and aligned.
Network remains disabled, source mounts read-only, no-new-privileges and
capability drop remain intact, and the original full/all-target test scope
never automatically falls back. The published Git import evidence records the
actual `docker_memory_limit_mb`. A separate run must prove the higher-memory
contract-focused case; any default nightly-only benchmark failure continues
to block a full/all-target compatibility claim.
