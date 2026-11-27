# Independent security review evidence package — not an approval

**Candidate:** Composer external Git importer and offline container verifier, based on [PR #36](https://github.com/KUNDURUHIMANEESHREDDY/composer/pull/36). This package helps an **independent human reviewer** verify the relevant boundaries; its author is not an independent reviewer.

## Upstream capabilities reused

- Git CLI for pinned clone revision and effective configuration, including refusal of checkout-controlled fsmonitor/clean/smudge/process helpers before `git status`.
- Cargo native locked Git dependency resolution and compiler verification against independently authored target-owned tests.
- Tree-sitter Rust for bounded source signature discovery (not semantic equivalence).
- Independently maintained Google gVisor `runsc`, installed from SHA256-pinned upstream bundle on Linux, as an optional fail-closed Docker runtime.
- RustSec advisory scanner, including the previously fixed `url 2.5.8` advisory.
- Independent [CodeQL Rust static analyzer](https://github.com/github/codeql-action) pinned to `bf58c2ac1f8060aa73f1225f461c80391348076a`, configured with upstream `security-extended` queries, no source compilation and SARIF evidence. No local scanner implementation.

## Human-review attack surface checklist

| Boundary | Source to inspect | Evidence / reviewer action |
|---|---|---|
| Git metadata host execution | `crates/integration/src/upstream_git.rs` | Verify effective config scan precedes Git status; independently test checkout-local includes, conditional includes, fsmonitor, filters, non-UTF8 Git settings, and host command execution. Do not assume all Git config mechanisms were enumerated |
| Source origin/provenance | `crates/integration/src/upstream_git.rs` and prior stacked PRs | Verify full 40-character SHA, clean checkout, exact GitHub HTTPS source URL, expected Cargo package name and license; inspect untrusted `.git` metadata, symlinks, submodules, Git object integrity and local cache trust |
| Cargo build code | `crates/integration/src/verified.rs` | Review build scripts and proc macros as malicious code, offline cache assumptions, read-only mounts, container/user permissions, timeouts, process cleanup, published stage/rollback |
| gVisor runtime registry | `scripts/install-pinned-gvisor.sh` and Docker workflow | Review release SHA256 pin, tar entries and sidecars, malicious daemon/runtime alias, update/CVE procedure; no guarantees against kernel/runtime escape |
| Secret exposure | CLI/logging code and GitHub Actions | Inspect error logs and Git credential handling; verify no credential-bearing Cargo home or arbitrary worktree mount to the container |
| API inference & integrity | `api_discovery.rs`, `reviewed_upstream_adapter.rs`, `api_signature.rs` | Verify no arbitrary Rust expressions are emitted. Same-signature different-behavior negative controls must fail and originals remain unchanged |
| Dependency & release provenance | `Cargo.lock`, RustSec workflow and separate Windows installer PRs | Check known vulnerabilities, third-party licenses, signed installer requirement and reproducible install/uninstall at exact release SHA |

## Independent security acceptance rules

1. Archive the **exact commit SHA** and the complete CI evidence. Do not mix successful checks from older branch heads with the final release candidate.
2. Require CodeQL Rust scan to execute, publish SARIF and report unresolved findings. A green scan cannot prove absence of vulnerabilities.
3. Require independent human review by a contributor who did not author or implement the changes. Have the reviewer record their GitHub identity, scope, specific evidence, open findings and a dated decision on the PR or [release gate #35](https://github.com/KUNDURUHIMANEESHREDDY/composer/issues/35).
4. Reject production/release sign-off for any unresolved exploitable finding, missing Windows Docker Desktop runtime evidence, missing provenance, or inadequate unfamiliar-repository completion metrics. Do not weaken tests to make CI pass.

### Acceptance status

- **Automated Git/Cargo/typed adapter regressions:** already passed on their individual exact branch heads; see PR #36 and the stacked PRs.
- **Third-party CodeQL Rust scan:** pending until its own exact-head CI completes and findings are inspected.
- **Independent human source/security audit:** **not performed / reviewer not assigned**.
- **Windows Docker Desktop gVisor execution:** **not verified**; the user's Windows device was connected but its Linux engine was stopped and memory was low.
- **Production security approval:** **not granted**.

This file is a handoff document, not an independent audit report or proof of full containment.
