# External gVisor runtime for Composer verification

Composer uses an independent runtime from https://github.com/google/gvisor instead of implementing its own operating-system sandbox. gVisor provides the Docker-compatible runsc OCI runtime. Composer's existing Cargo/third-party code verification remains the same; only the runtime selected when launching Docker changes.

Install and register runsc following the official gVisor instructions:
- https://gvisor.dev/docs/user_guide/install/
- https://gvisor.dev/docs/user_guide/quick_start/docker/

On a Linux host with Docker configured for runsc, set COMPOSER_VERIFIER_RUNTIME=runsc in the environment when invoking Composer's existing verify-external-git-rust, verify-external-rust-crate, or other verifier commands.

A request for runsc passes --runtime=runsc directly to Docker. Missing or misconfigured runsc causes execution to fail; Composer does NOT fall back to runc. Unset COMPOSER_VERIFIER_RUNTIME retains current Docker defaults for backward compatibility. Other environment values, including the empty string and invalid encoding, are rejected.

Existing protections (network=none, non-root, read-only filesystem, dropped capabilities, no-new-privileges, resource limits, read-only source mounts and bounded time) remain in the shared verifier. The returned build log is prefixed with the requested Docker runtime name to make the evidence auditable. This is a record of the request, not an independent cryptographic attestation of the Docker daemon's actual binary.

The dedicated GitHub Actions workflow external-gvisor-sandbox.yml installs the signed upstream gVisor APT package, records its package version and Docker runtime registration, runs a direct smoke test, and executes the pinned Git deunicode-to-slugify integration benchmark using runsc. The source algorithm stays upstream. Three success cases and failure cases for wrong semantics, invalid signatures and incorrect Git SHA must still pass their independent tests before acceptance.

Limitations: gVisor is neither a virtual machine nor a guarantee that malicious code cannot escape. This integration does not pin or cryptographically attest an individual gVisor binary build; the CI verifies the signed APT distribution and records its installed version. Treat a compromised Docker daemon or runtime alias as outside the asserted trust boundary. Linux gVisor execution is separate from native Windows Rust tests, and Docker Desktop requires its own supported gVisor configuration. Code origin and license review, source-to-artifact provenance and independent behavior tests remain mandatory.
