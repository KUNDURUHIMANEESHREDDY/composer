# Import an existing GitHub Rust capability with Cargo (no source rewriting)

For an independently reviewed public Rust library that is not available on
crates.io, Composer can use Cargo's native **Git dependency** implementation
instead of copying functions or reimplementing their algorithms.

Command: \`composer verify-external-git-rust\`.

Unlike the published-registry reuse path, it verifies a specific **Git
commit**, not a crates.io archive. Only canonical HTTPS GitHub URLs and full
40-hex commit IDs are allowed; floating branches, tags and release labels
are not accepted. The source checkout must have the same HEAD commit,
matching GitHub origin, and no uncommitted/untracked files. Its license
declaration and an original license file must be present.

## Preparation (no host-side code execution)

The user reviews the code and license in an independently acquired checkout.
Provision the already-existing Cargo Git cache from a disposable manifest
using **cargo fetch**, not cargo build or cargo test. For example:

\`\`\`toml
[dependencies]
composer_git_strsim = { package = "strsim", git = "https://github.com/rapidfuzz/strsim-rs.git", rev = "dacc84c0dc61eff0ee0ff66962bcf2e17018ad26" }
\`\`\`

\`\`\`sh
cargo fetch --manifest-path disposable-prefetch/Cargo.toml
\`\`\`

Review and run the command:

\`\`\`sh
cargo run -p composer-cli -- verify-external-git-rust path/to/target \
  --output path/to/new-verified-target \
  --upstream-checkout path/to/clean-reviewed-checkout \
  --upstream-repository https://github.com/rapidfuzz/strsim-rs \
  --upstream-commit dacc84c0dc61eff0ee0ff66962bcf2e17018ad26 \
  --package strsim --public-symbol jaro --target-alias ported_jaro \
  --contract-test tests/independent_git_contract.rs \
  --registry-cache "$HOME/.cargo/registry" \
  --git-cache "$HOME/.cargo/git" \
  --license-reviewed
\`\`\`

Composer stages a **new target copy**, uses an isolated namespaced Cargo
dependency (\`composer_git_strsim\`), and leaves any existing registry
\`strsim\` dependency untouched. The Git and registry caches are mounted
**read-only** in the existing network-disabled, resource-bounded Docker
Cargo verifier. Nothing from the upstream checkout is compiled or run
by Composer on the host. Docker runs the independently authored target tests.

Publication requires:
- A passing Cargo offline compilation and target-owned behavioral test.
- Exactly one Cargo.lock Git package whose source matches the requested
  repository **and** exact commit hash.
- A matching clean reviewed Git checkout and original SHA-256-hashed license
  evidence stored in \`third_party_licenses/\`.
- A new output directory, preserving the original project and returning
  error/no result if any check fails.

The published result includes \`composer-upstream-git-evidence.json\`.
The Git hash ties the checkout and Cargo's resolved source to the requested
commit, but this **does not** certify upstream reputation, licensing
compatibility, build-script safety against container escape or complete
semantic equivalence. Docker is not VM-grade isolation.

This initial implementation verifies direct root-package public functions
or simple nested paths. Unpublished monorepo subpackages requiring Cargo
\`subdir\`, private authentication, non-Rust plugins, Git submodules and
arbitrary dependency adaptation remain unsupported. Windows end-to-end
testing remains open.
