# External-source adapter and gVisor security review handoff

This document is a **review package**, not an independent security audit or a certification.

## Scope and trust boundaries

Composer reuses existing, independently maintained tools:
- Rust parsing: upstream Tree-sitter grammar and parser, already integrated in Composer.
- Dependency resolution and provenance: Cargo's pinned Git resolver, the source checkout's exact SHA, and Cargo.lock.
- Execution boundary: upstream gVisor `runsc`, pinned upstream release archive SHA-256, accessed through Docker's OCI runtime option.

The original algorithms remain in their upstream repositories. Composer generates only narrow typed adapters (owned `String` arguments borrowed as `&str`) and does not generate source implementations. Independently authored tests must pass before publishing a separate target copy.

The additional [RustSec audit-check](https://github.com/rustsec/audit-check) action is consumed at pinned upstream commit `69366f33c96575abad1ee0dba8212993eecbe998` and checks Cargo.lock against RustSec advisories. This is a third-party **dependency vulnerability scanner**, not an independent review of the Composer source code or gVisor containment. Any security findings must be examined and remediated using safe updated upstream dependencies rather than ignored to turn CI green.

### Reviewed protections and failure cases

| Threat | Existing or new check | Still unproven |
|---|---|---|
| Search result chooses user's own or ambiguous source | Checked GitHub owner and pinned reviewed source; unknown/ambiguous candidates rejected | Cross-repository semantic discovery |
| Tricked parser or injected Rust expressions | Tree-sitter AST must identify one supported public top-level function; restricted symbol/alias; no free-form expressions | Parser implementation vulnerability, edge-case Rust syntax |
| Compile succeeds but result is wrong | Independent target-owned contract rejects known wrong-but-compiling algorithms; tests include Jaro vs Jaro-Winkler | General semantic equivalence and unseen inputs |
| Wrong or mutable upstream Git revision | Clean exact checkout at specified 40-digit SHA; Cargo.lock source revision checked | Signed upstream commit, author/maintainer integrity |
| Checkout-controlled host Git command execution | Host `git status` cannot invoke `core.fsmonitor` (native Git override) or `filter.*.clean/smudge/process` (effective local/included Git config scanned and refused before status); global/system Git config and inherited config overrides excluded | Other host-side Git command/config surfaces require independent review |
| Malicious build scripts and package code | Restricted Docker with no network, no-new-privileges, non-root, read-only source and rootfs, CPU/memory/pids limits; optional gVisor `runsc` | Escape resistance, compromised Docker daemon or runtime alias |
| Supply-chain substitution of sandbox runtime | GitHub-published upstream gVisor bundle pinned to SHA-256 and verified before extraction; binary digest recorded | Independent signing/attestation and timely CVE patching |
| Windows Desktop configuration | Native locked Rust build and PowerShell syntax test; optional read-only Docker Desktop/runsc host probe | Actual gVisor execution in user's Windows Docker Desktop |

### Security acceptance before merge

A human reviewer independent of this implementation should inspect the exact upstream source and licenses, pinned release digests, Docker mount/read-only boundaries, parser and adapter restrictions, build-script capabilities, runtime registry trust, dependency recovery behavior, and whether error paths leave staged files or modified originals. Reject elevated Docker flags, runtime downgrade, unpinned external dependencies, and any evidence derived only from compilation without target-owned assertions.

Run Windows Docker Desktop acceptance on a separately authorized Windows machine:

```powershell
pwsh -NoProfile -File scripts/check-windows-docker-gvisor.ps1
```

The script does not download anything or modify the Docker daemon. It checks the active Linux daemon, registered `runsc`, an already-cached Rust image, and an isolated no-network smoke run, then writes a small JSON record. Nonzero exit or missing Docker Desktop is **not verified**, not a pass. A passing smoke test alone would still not demonstrate a complete third-party Git import on that Windows setup.

### Outcome

**Observed Windows Desktop Commander preflight, 27 November 2026:** the authorized Windows computer was connected and Docker CLI, Rust Cargo and Git were present, with Docker context `desktop-linux`. However the Docker Desktop Linux engine pipe was absent and `docker desktop status` reported the application not running. Windows reported only approximately 565 MiB of available physical memory, so Docker Desktop was not force-started or other applications terminated. This is a **failed/not-run live Windows gVisor acceptance gate**, not a runtime success or independent security review.

**No independent external security assessment was performed by this PR.** Passing the Linux gVisor and Windows native CI does not establish general hostile-code containment, and extending return types to `bool` or scalars does not establish semantic correctness. Do not label Composer production-hardened on this evidence alone.
