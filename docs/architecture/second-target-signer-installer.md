# Composer: second target, signer check, Windows installer

Composer reuses the existing Euler totient implementation from external TheAlgorithms/Rust, pinned at commit 976f1cdcdab3fab85301c38b28c28e6c5c503e9a, in another independent public Rust target: weihanglo/rust-algorithm-club, pinned at 1954301f21d44c18ea44864a776a3f6fd72eaefe. The first verified target remains ijt/trigram. No upstream algorithm code is rebuilt from scratch.

The second target is an older Cargo library whose original lockfile pins an outdated libc version. Composer's existing Cargo dependency solver is allowed to resolve a compatible version in a disposable integration copy. The original target remains unchanged. Independent behavioral tests run inside the unchanged offline Docker verifier, and a wrong but type-compatible function must be rejected.

## Optional stronger upstream signing identity

The Git-only CLI accepts --require-upstream-signer-fingerprint followed by an independently reviewed 40- or 64-hex OpenPGP signing-key fingerprint. With the option, it uses Git's existing verify-commit, signature status, and signer fingerprint information. A commit must have a good, trusted signature and match the exact expected fingerprint. Unsigned, unknown-trust or wrong-fingerprint commits are rejected before building or publishing anything. The output evidence records whether this optional signature requirement was actually fulfilled. A clean Git checkout or matching commit SHA alone does NOT imply a trusted signer. Tests explicitly reject an unknown signer while preserving original files.

This option requires the operator to establish trust in the signing key by independent means and configure a trusted keyring. It is not automatically enabled for the curated public examples because their signer identities were not independently authenticated.

## Existing open-source NSIS packaging and clean-machine verification

A real Windows installer is built by NSIS 3.13.0, using the existing public Chocolatey tool package and packaging/composer-cli.nsi. The installation is per-user in the user's LocalAppData Programs/Composer folder and requires no elevation. The bundled standard NSIS uninstaller deletes only Composer's executable, uninstaller, and its HKCU uninstall registration; it does not recursively delete arbitrary user files.

GitHub Actions compiles the CLI with the checked-in Cargo.lock, uses NSIS to build composer-setup.exe, and attests the installer with GitHub's existing Sigstore-backed actions/attest mechanism. The installer and SHA256 digest are uploaded as temporary workflow artifacts rather than published as a release.

On a second clean Windows runner, the acceptance script verifies the GitHub attestation, checks the installer digest, silently installs Composer, executes the installed CLI integration subcommands, silently uninstalls it, and verifies the executable and registry entry were removed while a separate canary file remained intact.

Known limitations: GitHub Actions Windows testing is not testing on the user's local PC. The installer is not Authenticode-signed, and its build attestation is not an endorsement of upstream source maintainers. Two target repositories cannot establish arbitrary-site generalization; malware-grade isolation, independent signer identity review, and production releases require further work.
