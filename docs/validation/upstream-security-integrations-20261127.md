# P4 upstream security integrations — review evidence (2026-11-27)

This is a **security hardening candidate**, not a completed independent audit or release approval. The new worktree starts at the exact P0 branch head `0d75c99b41d8744a57812c7a6e67daf4b2996a9c`. No existing verification engine is replaced.

## Third-party security tools in the existing pipeline

| Mechanism | Source and purpose | Scope |
| --- | --- | --- |
| gVisor | Existing exact-version Docker `runsc` verifier | Linux sandbox CI; live Windows Desktop Docker remains unverified |
| RustSec | Existing SHA-pinned `rustsec/audit-check` | Advisory check for committed Cargo.lock |
| Syft | Existing SHA-pinned `anchore/sbom-action` | SPDX 2.3 SBOM in Windows portable distribution |
| GitHub artifact attestation | Existing SHA-pinned `actions/attest`, verified by `gh attestation verify` | Distribution provenance; not publisher identity verification |
| CodeQL | `github/codeql-action` at `bf58c2ac1f8060aa73f1225f461c80391348076a` | Rust `security-extended` without running untrusted Cargo build scripts |
| cargo-deny | `EmbarkStudios/cargo-deny-action` at `bb119f768ea239e765aeaef5769558d4f21213aa` | Banned dependencies, license expressions, registry/Git source policies |
| OSV-Scanner | Official reusable workflow at `32d3cd482bdadfb38b186c0ad1b031902d55d932` | Vulnerability scan of Composer's Cargo.lock; fail if vulnerabilities or scanner errors |
| OpenSSF Scorecard | `ossf/scorecard-action` at `23f22cd43608bae7a1f1fa2603d7314dee9538a5` | Repository health/trust evidence; does not authorize any upstream source automatically |

The added actions and reusable workflow are pinned to their reviewed Git revisions. **That does not prove their bundled Docker images and transient dependencies are immutable.** Review the OSV image and cargo-deny action image supply chain separately. The new scanners run on GitHub-hosted CI and are limited to same-repository PRs for write-capable operations. These are monitoring and admission checks, not a substitute for runtime containment.

## Observed local results

- The official cargo-deny 0.22.2 Windows binary from the upstream release was SHA-256-checked against its release checksum before running. This is a checksum comparison, **not a verified signature or independent release attestation**.
- Initial scan found path-only workspace dependencies counted as wildcard version requirements. Adding `version = "0.1.0"` to the four workspace-local dependency declarations and the CLI-to-API path dependency kept the same paths while enforcing a compatible internal crate version. After this change: **bans PASS, sources PASS**, and `cargo metadata --locked --offline` PASS.
- **Licenses FAIL:** `composer-api`, `composer-cli`, `composer-core`, `composer-integration`, `composer-intelligence`, and `composer-repository` have no Cargo manifest license expression. No license was invented, no finding suppressed, and `deny.toml` retains a strict allowed SPDX list. The repository owner must select/document a valid project license and update manifest metadata before the full cargo-deny job can pass.
- CodeQL, OSV, and Scorecard workflow execution require exact-commit GitHub CI evidence; local YAML syntax and immutable action pin tests do **not** establish those scans passed.

## Remaining release blockers

1. Determine and apply the project-owned license to the six first-party packages; verify license text and distribution policy.
2. Run every scanner at the exact combined PR SHA; inspect artifacts and remediate findings without global ignore rules.
3. Validate the new scanner actions' runtime dependencies and Docker images, not just the Git commit pins.
4. Verify gVisor on an actual Windows Docker Desktop Linux engine and run a real third-party integration through `runsc`.
5. Complete the held-out 30-target functional gate and independent human threat-model, sandbox, supply-chain, credential and release review (see issue #35).
6. Decide and document signing trust identity: existing GitHub Sigstore build attestation is verified; Cosign should be added only for a concrete separately signed artifact and a pinned, trusted issuer/identity policy. Do not create self-signed green checks.

**Do not merge solely on automated scanner success.** Independent security signoff and installation/release gates remain mandatory.

## First hosted OpenSSF Scorecard findings (PR #43, commit `c788f6c`)

The [Scorecard run](https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/46037946600) completed successfully and produced a six-result SARIF artifact. **Scanner execution success is not a passing security score.**

- `SecurityPolicyID` score 0: `.github/SECURITY.md` has now been added; require a fresh scan before marking remediated.
- `DependencyUpdateToolID` score 0: `.github/dependabot.yml` has now been added for weekly reviewed Cargo and GitHub Actions update PRs; require a fresh scan before marking remediated.
- `LicenseID` score 0: unresolved; repository owner must explicitly select a valid license.
- `FuzzingID` score 0: unresolved; do not claim coverage from independent functional tests alone.
- Two `TokenPermissionsID` results score 9: justified writes in individual jobs still merit review; no broad token elevation was added.

The same head's hosted [OSV-Scanner job](https://github.com/KUNDURUHIMANEESHREDDY/composer/actions/runs/46037947006) passed. The `cargo-deny` job failed, consistently with missing first-party licensing metadata. Do not mask this failure by relaxing the policy.
