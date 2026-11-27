# Recover third-party capabilities without reimplementing them

The \`composer recover-external-rust-crate\` command automates **bounded
selection and verification** across an explicitly reviewed list of independent
public Rust crate implementations. Composer uses existing Cargo's package
solver and registry metadata, never generates a replacement algorithm.

## What recovery actually does

1. Examine the target's **existing** independent tests, without modifying it.
2. For each reviewed candidate (maximum 5), resolve at most 3 stable versions
   already present in the local crates.io Cargo registry cache, newest first.
   \`latest\` means **latest cached stable version**, NOT latest online.
3. If the target already declares the candidate package, introduce an isolated
   Cargo dependency alias, such as \`composer_strsim\`, pinned to the selected
   version. This leaves the target's original requirement untouched and relies
   on Cargo to reject incompatible dependency graphs. It does **not** silently
   upgrade existing dependencies.
4. Try to re-export that crate's public function, then run existing target tests
   in Composer's restricted offline Docker verifier. A compilable implementation
   with incorrect semantics fails the behavioral tests and will not be published.
5. If a candidate fails, try the next cached version and reviewed candidate,
   up to 10 total attempts. Only publish a new output copy upon independent
   verification, with exact Cargo lockfile/checksum evidence. If all candidates
   fail, return an explicit \`completed: false\` report and publish nothing.

## Candidate file

Create \`candidates.json\` from independently reviewed external projects:

\`\`\`json
[
  {
    "package": "strsim",
    "version_requirement": "^0.11",
    "public_symbol": "jaro",
    "upstream_repository": "https://github.com/rapidfuzz/strsim-rs",
    "upstream_commit": "dacc84c0dc61eff0ee0ff66962bcf2e17018ad26"
  },
  {
    "package": "textdistance",
    "version_requirement": "1.1.1",
    "public_symbol": "str::jaro",
    "upstream_repository": "https://github.com/life4/textdistance.rs",
    "upstream_commit": "0f92088dfc84a99a7652b3f6d7e268afd98dbaa6"
  }
]
\`\`\`

The first candidate may instead offer \`jaro_winkler\`, which is a different
algorithm despite an identical \`(&str, &str) -> f64\` signature; independent
contract tests must catch this and force fallback.

\`\`\`sh
cargo run -p composer-cli -- recover-external-rust-crate path/to/target \
  --output path/to/recovered \
  --candidates-json candidates.json \
  --target-alias ported_jaro \
  --contract-test tests/ported_jaro.rs \
  --registry-cache "$HOME/.cargo/registry" \
  --license-reviewed
\`\`\`

The target must already contain a \`#[test]\` in the named contract file
asserting \`ported_jaro(...)\`. The caller needs to review **every** candidate's
source and license; \`--license-reviewed\` is an attestation, not clearance.
Before running recovery, prefetch the exact candidate artifacts using
\`cargo fetch\` from a separate throwaway Cargo project (do not execute
untrusted upstream build scripts on the host).

## Explicit limits

- **Candidate discovery is not automatic internet-wide search.** The user,
  caller or another tool must supply and review the independent candidates.
- This workflow supports registry-published Rust crates, not arbitrary Git-only
  libraries, npm/Python packages or non-Rust APIs.
- Rust's compiler, Cargo's dependency resolver and target tests, not semantic
  name similarity, determine whether a candidate is accepted. Independently
  authored tests are only as complete as their cases.
- Candidate source URLs and upstream commit SHAs are provenance **claims** and
  are not cryptographically proven to correspond to the crates.io archive;
  Cargo checksum verifies the locked registry package.
- Alias-based dependency isolation does not solve native \`links\` conflicts,
  disallowed duplicate dependencies or unsafe build scripts. Such attempts
  must fail closed. The Docker verifier is not VM-level isolation.
- Windows validation and broad unfamiliar-source success statistics remain open.
