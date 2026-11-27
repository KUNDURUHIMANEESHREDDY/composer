# Composer independent security-review handoff — 2026-11-27

Status: **NOT independently audited or approved.** This is a review request, not an independent auditor's opinion. An unaffiliated reviewer must document the reviewed commit, findings, retests and sign-off.

## Scope
- crates/integration/src/verified.rs — Docker execution boundary, mount restrictions, image ID, resources, Cargo commands and cleanup.
- crates/integration/src/reviewed.rs — reviewed-rule admission, test discovery/execution assurance, syntax and patching.
- crates/integration/src/api_discovery.rs — type-shape fallback, rank thresholds, ambiguity and revalidation.
- crates/integration/src/{verify,wire,apply,multifile}.rs — capsule hashes, paths, staging and publication.
- crates/repository/, crates/intelligence/, Cargo.lock, .github/workflows/ — untrusted inputs, dependencies and CI.

## Changes requiring review
1. Type-compatible name-unrelated functions can be shown as **score-10 suggestions** only. They cannot be auto-selected; an explicit current-rule match plus isolated compiler and independent contract tests are still mandatory.
2. Cargo manifest test overrides are now refused for reviewed contracts, including custom test targets whose harness could be disabled or whose required-features could skip testing.
3. The contract-test scanner now demands a directly called target as the first assertion argument and rejects known spoofing patterns including nested macros, uncalled closures, branch-guarded assertions, empty loops and direct function-to-itself comparisons. This remains a syntactic admission screen, not independent proof of every test's execution.

## Required external verification
- [ ] Clean pinned-commit formatting, Clippy and workspace tests.
- [ ] Attempt to bypass admission via Cargo autotests=false and explicit [[test]] harness=false/required-features/path settings; no output may be published.
- [ ] Try ignored, conditionally compiled, expected-panic, stringified, unexecuted or vacuous tests; report any acceptance.
- [ ] Deliberately insert incorrect target implementations; independently authored behavior tests including boundary cases must reject them in isolated builds.
- [ ] Challenge container escape, Docker daemon access, host secrets, environment variables, mounts, symlink races, metadata/network egress, resource exhaustion, timeout and cleanup.
- [ ] Independently inspect dependency advisories, registry and cache provenance, build-image signatures, SBOM and supply-chain integrity.
- [ ] Test repeated, unfamiliar third-party source/target integrations under an explicit success protocol, and collect fail cases.
- [ ] Test a clean Windows installation and the isolation backend actually in use.
- [ ] Deliver unaffiliated reviewer findings, severity, reproductions, remediations, retest evidence and signed conclusion.

## Test commands (must record execution outcomes)
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    python tests/test_reviewed_api_repeated_public.py

The Python behavioral run requires Docker, the pinned image and offline Cargo cache. A Rust unit-test pass is not evidence that these Docker scenarios ran.

## Automated security-gate integration (not an external audit)
- GitHub's upstream dependency-review action was attempted but its GitHub dependency-diff API returned HTTP 403 both in CI and with the authenticated repository owner's CLI. This was an unavailable API, **not** an identified vulnerable dependency. The unusable workflow was replaced with upstream RustSec `cargo-audit` (installed using taiki-e/install-action with checksums, fallback disabled). This read-only workflow audits Composer's Cargo.lock from outside the untrusted checkout and is also scheduled weekly once merged. It reports **known** crate advisories, not runtime isolation, publisher provenance or unknown supply-chain vulnerabilities. Its hosted check must pass before the gate is considered operational.
- The review still requires an unaffiliated security assessor and an explicit release decision.

## RustSec finding and remediation
- The first upstream RustSec 0.23.0 audit of Cargo.lock failed on RUSTSEC-2026-0592 affecting url 2.5.8 (nested blob-origin processing). No advisory suppression was added.
- Updated the lockfile to url 2.5.9 (patched) and form_urlencoded 1.2.3. The complete native Rust workspace tests and strict Clippy passed after this update. A fresh hosted RustSec audit and full Docker acceptance must confirm the final commit.

## Preliminary automated scanning (not an external audit)
- Semgrep 1.181.0 with the public p/rust ruleset reported **10 temp_dir findings** across Rust unit-test fixtures. All reported call sites are inside cfg(test) sections and use UUID-suffixed directories; they are **untriaged scanner findings**, not a clean scan or an external security conclusion.
- Windows Docker daemon is unavailable (Linux engine named pipe missing). Only approximately 1.6 GiB of RAM was free when checked. Do not infer Docker behavioral results from host compilation.

## Open limitations
- A Docker image ID is not signed publisher provenance. Docker shares the host kernel; container flags do not certify hostile-code isolation.
- Static syntax admission does not prove meaningful assertions or reachability in every case.
- General-purpose semantic equivalence of arbitrary APIs is not proven, and the current supported Rust subset is narrow.
- Unreviewed dependency behavior, malicious build scripts, kernel escapes and supply-chain attacks remain potential risks.

Independent reviewer: **pending**
Production security sign-off: **not granted**
