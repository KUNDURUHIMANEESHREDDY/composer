# Find external capabilities before writing implementation code

Composer now has an experimental **read-only external package search** backed by Cargo's maintained crates.io search. It is intentionally separate from integration and publication.

```powershell
cargo run -p composer-cli -- discover-external-crates "luhn" --limit 10
cargo run -p composer-cli -- discover-external-crates "jaro similarity" --limit 10
```

The command validates the user query, runs `cargo search --registry crates-io` with argument boundaries (no shell), and returns JSON containing package name, registry-listed version, description, and deterministic **lexical** relevance. The query cannot invoke third-party build scripts. It does not create a project, modify a target, or select a source to execute.

**It does not automatically prove** the corresponding repository, upstream license, supply-chain integrity, API semantic match, or safe execution. Registry text is untrusted input. Candidates may be stale or misleading, and the query needs network access.

## How to reuse a selected external library

1. Identify the independent source project's official repository and inspect its license, current security posture and public API. Verify it isn't an owned project.
2. Choose an exact published version and confirm it has a matching registrable Cargo artifact. Pin source Git revision separately; the existing package-adoption workflow reports that a supplied Git commit is **not** cryptographically matched to the published artifact.
3. Supply independently authored target behavior tests, including edge cases and a wrong-but-compiling negative case.
4. Use `verify-external-rust-crate` to import the published library into a fresh target copy. Cargo resolves the existing dependency and checksum; Composer only adds a reviewed re-export adapter. No replacement algorithm should be generated.
5. Publish only when network-disabled restricted Docker tests pass, the lockfile checksum exists, provenance claims and limitations are recorded, and the original target is unchanged.

The external-crate import currently supports a narrow reviewed `pub use upstream::symbol as alias;` shape, not arbitrary traits, methods, integrations or automatic code fixes. A candidate from `discover-external-crates` is **never** treated as successful composition.

## Evidence to request

- Number of registry candidates found and their immutable exact package versions
- Upstream repository, license, SHA, package registry checksum and mismatch/provenance caveats
- Independent executable behavioral contracts
- Successful and unsuccessful isolated tests, including an intentionally incorrect implementation
- Fresh-target repetition count and absence of mutations on failure
- Windows verification separately from Linux Docker testing

For the initial Luhn and Jaro use cases, see the existing separate public-source and published-crate benchmarks.