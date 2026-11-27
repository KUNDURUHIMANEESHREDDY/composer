# Reusing more than one independent Git-only Rust project

Composer's published-crate reuse and Git-only reuse are **not** new algorithm
implementations. Both rely on Cargo's existing dependency solver and the
original upstream code. This acceptance milestone covers two different Git-only
upstream repositories, different capabilities, and unrelated target projects.

## Sources and proof

- \`TheAlgorithms/Rust\`, SHA \`976f1cdcdab3fab85301c38b28c28e6c5c503e9a\`,
  \`the_algorithms_rust::number_theory::euler_totient\` into
  \`ijt/trigram\` at \`45af254dcd7b5c8aac4feb336192204791cd1e2f\`.
- \`weihanglo/rust-algorithm-club\`, SHA
  \`1954301f21d44c18ea44864a776a3f6fd72eaefe\`,
  existing optimized and naive \`levenshtein_distance\` functions into
  \`rapidfuzz/strsim-rs\` at
  \`dacc84c0dc61eff0ee0ff66962bcf2e17018ad26\`.

The CI acceptance requires independent target-owned semantic assertions
and fail-closed rejection of a wrong but type-compatible function, exact
Cargo.lock Git revision matching, and unchanged original target bytes.

The source/license provenance evidence adds Git's **tree object ID** and
verifies that the actual \`Cargo.toml\` and source \`LICENSE\` contents match
the pinned Git object's blob IDs using Git's own \`hash-object\` and
\`rev-parse\` rather than reimplementing a hashing protocol. These IDs are
included in the recovered output evidence and response. This verifies
consistency with the reviewed Git commit, **not** the upstream's reputation,
maintainer identity, licensing compatibility or that arbitrary code is safe.

## Windows portable artifact and provenance

The \`.github/workflows/multisource-attested-cli.yml\` workflow reuses
\`actions/attest@v4\`, \`actions/upload-artifact@v4\`, and
\`actions/download-artifact@v4\` (not a new signing system).

It runs native tests on clean Windows, compiles a **release** \`composer.exe\`
using the exact checked-in \`Cargo.lock\`, creates a portable ZIP with a
SHA-256 executable digest, attests the ZIP with GitHub's Sigstore-backed
artifact attestation, and publishes a *temporary Actions artifact*, **not** a
public GitHub release.

A second, separate clean Windows runner downloads that ZIP and verifies
the artifact provenance using the **existing GitHub CLI**
\`gh attestation verify\`, independently checks the SHA-256 digest and starts
the extracted executable to exercise the discovery, Git-only and recovery
CLI commands. This is a genuine packaged portable-executable smoke test,
but **not** an MSI/EXE installer test, a signed Authenticode Windows
installer or a test on the user's local computer.

The attestation proves a build workflow made the subject ZIP. It is not
a third-party source certification or general SLSA level claim.

## Remaining release gates

- Generalize beyond two curated public Git-only sources. Existing contracts
  prove specific behaviors and don't establish arbitrary semantic correctness.
- Review malicious-build-script isolation, dependency trust, native libraries,
  Git workspaces/monorepos, network credentials and source-provenance identity.
- Build a true Windows installer and verify install/uninstall, user profile
  isolation, file permissions and packaged end-to-end workflows on an
  independent clean machine.
