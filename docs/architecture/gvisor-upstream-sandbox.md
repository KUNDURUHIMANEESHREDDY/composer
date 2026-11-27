# External gVisor runtime for Composer verification

Composer uses an independent runtime from https://github.com/google/gvisor instead of implementing its own operating-system sandbox. gVisor provides the Docker-compatible runsc OCI runtime. Composer's existing Cargo/third-party code verification remains the same; only the runtime selected when launching Docker changes.

Install and register runsc following the official gVisor instructions:
- https://gvisor.dev/docs/user_guide/install/
- https://gvisor.dev/docs/user_guide/quick_start/docker/

On a Linux host with Docker configured for runsc, set COMPOSER_VERIFIER_RUNTIME=runsc in the environment when invoking Composer's existing verify-external-git-rust, verify-external-rust-crate, or other verifier commands.

A request for runsc passes --runtime=runsc directly to Docker. Missing or misconfigured runsc causes execution to fail; Composer does NOT fall back to runc. Unset COMPOSER_VERIFIER_RUNTIME retains current Docker defaults for backward compatibility. Other environment values, including the empty string and invalid encoding, are rejected.

Existing protections (network=none, non-root, read-only filesystem, dropped capabilities, no-new-privileges, resource limits, read-only source mounts and bounded time) remain in the shared verifier. The returned build log is prefixed with the requested Docker runtime name to make the evidence auditable. This is a record of the request, not an independent cryptographic attestation of the Docker daemon's actual binary.

The dedicated GitHub Actions workflow external-gvisor-sandbox.yml invokes scripts/install-pinned-gvisor.sh to retrieve the **exact unmodified upstream gVisor release-20261123.1 tar.bz2 bundle** directly from GitHub Releases. It checks SHA-256 against an independently recorded, immutable GitHub asset digest BEFORE extraction, rejects unsupported architectures/unsafe archive paths, confirms bundled shim and sidecars, records both the bundle SHA-256 and installed binary SHA-256, registers runsc with Docker, and verifies the requested runtime was used in three independent Git capability transfers. No floating APT version or newly implemented sandbox is involved.

Limitations: gVisor is neither a virtual machine nor a guarantee that malicious code cannot escape. The installer pins the upstream bundle SHA-256 for x86_64 and aarch64, but does not cryptographically attest the running Docker daemon or bind the runtime alias to a trusted binary. The pinned release must be reviewed for security updates; no automatic upgrades are permitted without a new digest and repeatable test evidence. Native Windows compilation is separate from Linux gVisor verification. Docker Desktop on Windows needs its own supported Linux-engine configuration and is NOT validated by the hosted Windows Rust tests. Code origin and license review, source-to-artifact provenance and independent behavioral tests remain mandatory.
