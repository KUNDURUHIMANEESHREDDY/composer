# External capability discovery by reusing Cargo and Composer's analyzer

Composer's \`discover-external-rust\` command uses **Cargo's existing crates.io
search and info commands**, Composer's existing Tree-sitter Rust repository
analyzer and its existing strict GitHub URL parser. Composer does **not**
reimplement the search engine or generate a replacement algorithm.

Example:

\`\`\`sh
cargo run -p composer-cli -- discover-external-rust jaro \
  --function jaro \
  --registry-cache "$HOME/.cargo/registry" \
  --exclude-owner YOUR_GITHUB_USERNAME \
  --limit 5 --output reviewed-candidates.json
\`\`\`

Cargo retrieves registry metadata and downloads published crate archives for
static inspection, but **does not compile or run source code on the host**.
The scanner proposes only functions with a visible \`pub fn\` or
\`pub async fn\` declaration in a supported public module path, rather than
mistaking a description or arbitrary symbol-name match for a capability. It
skips crates without an acceptable canonical GitHub origin, license metadata,
or a matching supported public function. Existing Cargo behavior handles
network requests and archive acquisition.

\`reviewed-candidates.json\` is a JSON array compatible with the already
implemented bounded recovery workflow:

\`\`\`sh
cargo run -p composer-cli -- recover-external-rust-crate path/to/target \
  --output path/to/verified-output \
  --candidates-json reviewed-candidates.json \
  --target-alias ported_jaro \
  --contract-test tests/ported_jaro.rs \
  --registry-cache "$HOME/.cargo/registry" \
  --license-reviewed
\`\`\`

The recovery command requires an **existing independent target-owned test** and
an explicit source/license review attestation, then uses exact-version Cargo
resolution and network-disabled Docker verification. On a rejected candidate it
can attempt alternatives; it publishes no output if all fail.

## Truthful provenance and limits

- \`cargo info\` supplies a **published registry version**, the advertised
  license expression and repository URL. These are third-party metadata claims.
- \`git ls-remote HEAD\` supplies an observed GitHub commit for that repository,
  **not** evidence that the published \`.crate\` archive was built from that
  commit. The version, Cargo.lock checksum and Git SHA are recorded separately.
- Discovery searches **crates.io**, not arbitrary GitHub repositories; Git-only
  projects are still unsupported. A registry package's GitHub URL alone does
  not prove the source-code correspondence.
- This is an upstream candidate shortlist, **not** automatic legal clearance,
  proof of semantic correctness, or a license policy decision. Even a function
  with the expected name may behave incorrectly; independent behavioral tests
  in Docker determine acceptance.
- Scanned public module paths are restricted (root function, public top-level
  module function), so exported aliases, trait methods and complex reexports
  are not automatically recommended.
- The cached registry contents and the user's target must still undergo
  software supply-chain review. The Docker verifier is not VM-grade isolation.
- Windows end-to-end verification remains separate from Linux CI and was not
  performed for this feature branch.

Public acceptance uses independent GitHub projects
\`rapidfuzz/strsim-rs\`, \`life4/textdistance.rs\` and
\`ijt/trigram\`, with the user's GitHub owner excluded at discovery time.
