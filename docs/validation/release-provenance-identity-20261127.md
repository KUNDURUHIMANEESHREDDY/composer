# Release provenance identity — Composer Windows artifacts (2026-11-27)

This change strengthens Composer's existing GitHub Sigstore-backed build attestations. It **does not** add another signing implementation or claim independent audit approval.

## Previous gap

Both Windows acceptance jobs verified `gh attestation verify --repo`, then checked either an adjacent SHA256 file or ZIP-internal build-info SHA. This verified repository-scoped provenance and artifact bytes, but did **not enforce** the expected signer workflow, exact signed source commit/ref, or originating workflow run and attempt. Build-info and adjacent checksum files are not independent identity assertions.

## Added exact-identity policy

The shared `scripts/verify-github-build-attestation.ps1` runs the **official GitHub CLI** to verify the artifact's SHA256-bound SLSA v1 attestation and enforces:

- fixed repository `KUNDURUHIMANEESHREDDY/composer`;
- allowlisted signer workflows (portable and NSIS installer only), passed as `--signer-workflow`;
- `--source-digest`, `--signer-digest` and `--source-ref`, from the current GitHub Actions environment;
- GitHub-issued OIDC identity and hosted-runner requirement;
- signed certificate's repository, signer URI, source SHA, ref and run invocation URI (including run attempt);
- the SHA256 of the **actual downloaded file**, compared with the verified statement's subject digest;
- one matching **single verified attestation**; reject missing, malformed or inconsistent evidence.

The strict policy runs on an **independent fresh Windows runner**, before ZIP extraction or installer execution. Existing packaging, checksum, NSIS install/uninstall, and source-SBOM checks remain unchanged. A synthetic negative-control test runs first to reject wrong repo/workflow/commit/ref/run/artifact hash and failed external verification; it is **not** a substitute for verifying a real signature.

**Important SHA distinction:** On `pull_request` workflows, `GITHUB_SHA` is often the temporary GitHub merge SHA (e.g. `refs/pull/<number>/merge`) rather than the feature branch head. The signed certificate binds this merge SHA, so acceptance must use the **current workflow environment's GITHUB_SHA/GITHUB_REF**, not PR head metadata. Do not hardcode an old SHA.

## Independent Cosign signer decision

The current signed attestation is verified using GitHub's Sigstore/GitHub OIDC trust policy. Running Cosign under the **same** workflow identity would generally duplicate, not independently increase, trust. Introduce an additional Cosign or Windows Authenticode signature **only after** documenting the separate signer/issuer, protected credential or OIDC identity, artifact subject and policy enforcement. Untrusted upstream source, compromised workflow code and malicious-but-validly-attested artifacts remain threats.

## Release gates

- Fresh exact-head CI must verify actual artifact signatures and intentionally reject incompatible identities.
- Preserve original functional and Windows clean-install acceptance checks.
- P4's unresolved first-party licensing, fuzzing, Windows gVisor execution, independent human security assessment, held-out compatibility and signing policy remain tracked at issue #35.
