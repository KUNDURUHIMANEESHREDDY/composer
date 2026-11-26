# Rust verification sandbox: implemented controls and remaining hardening

This document is an implementation and threat-model note, **not a claim that
Docker alone safely executes arbitrary malicious code**.

## Trust boundaries

- **Trusted control plane:** Composer's Rust CLI and verified capsule manifest
  handling run on the host. The host copies a bounded, non-symlink target
  snapshot; the original target and source capsules are left untouched.
- **Untrusted execution:** Cargo builds, build scripts, proc macros and target
  tests execute only in the isolated container, never directly on the host.
- **Source/license gate:** --license-reviewed is an explicit caller assertion.
  Composer cannot legally certify upstream licenses or audit arbitrary source.
- **Output:** successful builds publish only a new target directory with a
  resolved Cargo.lock and JSON evidence. Failed batch steps remove all
  intermediate directories and publish no requested target.

## Container restrictions (current)

- Local Rust image is inspected and invoked by its **exact image ID**;
  --pull=never blocks remote images at execution time. This records
  reproducibility information, but does not attest the image publisher.
- Networking disabled (--network=none); isolated IPC namespace.
- Read-only root filesystem, with separate bounded /tmp and /work tmpfs
  volumes. /work permits execution because Cargo tests require it.
- All capabilities dropped; no-new-privileges enabled; Docker's default
  seccomp policy applies where available.
- User is non-root: fixed 65534:65534 on Windows Docker Desktop, and the
  current non-root host UID/GID on Unix for safe file ownership and access.
- Process limit 64, CPU quota 1, memory limit 1024 MiB, file descriptor limit
  1024, total wall-clock timeout 240 seconds.
- Capsule and target input bind-mounted read-only. Optional Cargo registry
  cache is read-only; result directory alone is writable.
- Container is forcibly removed after success, error or timeout, and logs
  are bounded before inclusion in evidence.

## Dependency and repair policy

- Only direct Cargo dependencies expressed as version strings can be added.
- If the target already has a different string constraint, Composer retains
  it. After Docker resolves Cargo.lock, Composer checks the **selected locked
  version** against both semver requirements and rejects ambiguities.
- Missing or stale lockfiles may be regenerated **once**, but only after an
  explicit Cargo --locked error. Application test failures do not trigger
  retries.
- Multi-capsule composition is bounded to 2–5 distinct capability IDs, with
  independent tests and evidence after every step.

## What this does *not* protect against

- A kernel/container-runtime exploit; Docker is not a microVM.
- Malicious Cargo compiler plugins, build scripts or code executed inside
  the container exploiting kernel, runtime, CPU or filesystem vulnerabilities.
- Trusted control plane compromise, tampered local image, poisoned package
  cache, or a mistaken human licensing/security review assertion.
- Dependency behavior at runtime that tests do not cover, insecure target
  configuration, or hidden source-level behavior not captured by the capsule.
- Arbitrary cross-language adapters, multi-file rewrites, unsafe incompatible
  dependency migrations or general AI-driven repair.

## Production hardening acceptance criteria (not implemented)

1. Execute truly untrusted code in a fresh dedicated microVM or strongly
   isolated remote runner per job, with no host daemon socket mounted.
2. Pin and verify signed build-image provenance and allowed registries.
3. Use explicit seccomp/LSM profiles, isolated user namespaces, no host
   secrets, read-only source mounts, bounded temporary artifact storage.
4. Enforce source/license policy, dependency advisory review and package
   integrity checks before source execution.
5. Add adversarial tests for mount escapes, symlink races, resource exhaustion,
   malicious build scripts, network attempts and termination cleanup.
6. Add typed failure classification and bounded patch planning with
   independent post-repair verification before any automation is trusted.
7. Validate against unfamiliar real GitHub repositories, not only synthetic
   fixtures. Preserve original commits and include independent review.

## Repeatable acceptance tests

The script tests/test_verified_composition_e2e.py performs a controlled,
synthetic two-source composition and a second-stage failing test. GitHub Actions
executes it in Docker for same-repository branches, after lint/unit tests.
Docker image pulling is a separate, explicitly visible setup step; runtime
verification itself does not contact the network.
