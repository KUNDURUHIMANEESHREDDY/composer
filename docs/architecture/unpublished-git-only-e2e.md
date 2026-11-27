# Real unpublished Git-only capability acceptance

Composer now exercises a GitHub-only upstream implementation in addition to its
earlier crates.io adoption tests. The source is **TheAlgorithms/Rust**, pinned to
\`976f1cdcdab3fab85301c38b28c28e6c5c503e9a\`, and the unrelated target is
\`ijt/trigram\`, pinned to \`45af254dcd7b5c8aac4feb336192204791cd1e2f\`.

The source implements \`the_algorithms_rust::number_theory::euler_totient\`.
Composer does not recreate Euler's totient algorithm. It re-exports the
existing function from the original public Git commit through Cargo's native
Git-dependency resolver.

The test performs a live crates.io API check of the package name
\`the_algorithms_rust\` and requires HTTP 404 to confirm it is not published.
It then uses \`cargo fetch\` to pre-provision the reviewed Git commit without
executing upstream code on the host, and verifies the target-owned behavioral
test through the existing offline, resource-bounded Docker Cargo verifier.

Because the repository provides a committed MIT \`LICENSE\` file but no
\`package.license\` Cargo metadata field, the Git verifier now accepts a
reviewed, regular, nonempty and size-bounded license file from the exact
commit. It SHA-256-hashes and copies the original file into the verified
target. Explicit human license review remains mandatory.

The test also attempts a wrong but type-compatible upstream function
(\`math::factorial::factorial\`) and requires independent semantic rejection,
rejects a mismatch between reviewed commit and requested commit, and
rejects missing source/license review. Failed integrations must leave all
original target files unchanged and publish no output.

\`.github/workflows/unpublished-git-windows.yml\` includes an additional
**clean Windows GitHub Actions runner** test of Cargo unit tests and CLI
availability. This provides Windows **source-level** acceptance, not installed
MSI/EXE testing on the user's computer.

Known boundaries: one real external Git-only source doesn't establish
support for all GitHub repositories, monorepo subpackages, native code,
private Git credentials or arbitrary APIs. Docker isn't a VM-grade malicious
code sandbox, the Git commit doesn't prove upstream trustworthiness, and
unit tests don't certify complete semantic equivalence.
