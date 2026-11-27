# Software Composer

A Rust-first autonomous software composition agent. It aims to build applications by discovering, extracting, adapting, integrating, and verifying reusable capabilities from existing software repositories instead of rebuilding every capability from scratch.

> **Status: repository intelligence and safe capsule staging implemented.** The project has a Rust workspace, GitHub source acquisition, Tree-sitter parsing for six language variants, a versioned repository graph, conservative Feature Capsule extraction and integrity verification, direct dependency-manifest inventory, advisory target compatibility analysis, draft adapter contracts, and non-destructive capsule staging. Compiler-grade resolution, production adapter generation, package solving, and sandboxed builds/tests remain under development.

## Design principles

- Reuse existing implementations when they are suitable; do not optimize for copied-code volume.
- Prefer package reuse, then module reuse, service isolation, and finally code transplantation.
- Treat cloned repositories and their build scripts as untrusted input.
- Pin every source to an immutable revision.
- Track dependency closure, licensing evidence, transformations, and verification results.
- Use deterministic checks to validate agent decisions.
- Keep the control plane separate from isolated source-code execution.

## Workspace

| Path | Responsibility |
| --- | --- |
| `apps/api` | HTTP API for the composition platform |
| `apps/cli` | Local developer CLI |
| `crates/core` | Shared domain types and validation |
| `crates/repository` | GitHub acquisition and bounded manifest inventory |
| `crates/intelligence` | Tree-sitter parsing, repository graph, feature boundaries, Feature Capsules |
| `crates/integration` | Capsule integrity verification, dependency inventory, compatibility plans, adapter contracts, safe staging |
| `docs/architecture` | System design, intelligence model, MVP and threat model |
| `docs/adr` | Architecture decision records |
| `examples` | Example project specifications |

## Requirements

- Rust stable (edition 2024; minimum Rust 1.90)
- Git 2.x

## Quick start

Validate the example project specification:

```powershell
cargo run -p composer-cli -- validate-spec examples/project-spec.json
```

Acquire a public GitHub repository without running its code:

```powershell
cargo run -p composer-cli -- inspect https://github.com/serde-rs/serde
```

Analyze a local or acquired repository and save its machine-readable graph:

```powershell
cargo run -p composer-cli -- analyze .composer/sources/<checkout> --output .composer/analysis.json
```

Rank likely symbols for a capability:

```powershell
cargo run -p composer-cli -- recommend .composer/sources/<checkout> --capability-id authentication --description "Authenticate users and validate sessions" --limit 20
```

Build a Feature Capsule from one or more selected symbol IDs returned by analysis/recommendation:

```powershell
cargo run -p composer-cli -- capsule .composer/sources/<checkout> --capability-id authentication --description "Authenticate users and validate sessions" --seed-symbol <symbol-id> --output .composer/capsules/authentication
```

Verify capsule contents against the manifest (paths, file sizes, and SHA-256 digests):

```powershell
cargo run -p composer-cli -- verify-capsule .composer/capsules/authentication
```

Compare a capsule with a target repository and write the advisory compatibility plan:

```powershell
cargo run -p composer-cli -- compatibility .composer/capsules/authentication . --output .composer/compatibility-plan.json
```

Stage the verified capsule into a new target-local namespace:

```powershell
cargo run -p composer-cli -- integrate .composer/capsules/authentication .
```

Opt-in source-level wiring of a **reviewed standalone Rust** capsule into a Rust target:

```powershell
cargo run -p composer-cli -- wire-rust .composer/capsules/authentication path/to/target --license-reviewed
```

The flag records an explicit review assertion; Composer does not perform legal approval. This command requires exactly one Rust source file, existing matching direct dependencies, no unresolved/unfollowed references, no symbol collisions, and a Rust `src/lib.rs` or `src/main.rs` target. It creates `src/composer_<capability>.rs`, adds a module declaration to the target entrypoint, and preserves a `src/.composer_<capability>.root-backup` file. It **does not** run untrusted code, prove compilation, or install dependencies. Use an approved isolated build/test environment before trusting the integration.

Start the API:

```powershell
cargo run -p composer-api
```

Check health:

```powershell
Invoke-RestMethod http://127.0.0.1:3000/health
```

Validate a specification through the API:

```powershell
$body = Get-Content examples/project-spec.json -Raw
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:3000/v1/specifications/validate -ContentType 'application/json' -Body $body
```

## Intelligence capabilities

### Tree-sitter languages

The parser currently supports Rust, Python, JavaScript, TypeScript, TSX, and Go syntax. It extracts selected declarations, import statements, and direct call-expression names. Syntax errors are reported per file; source is not executed.

### Repository graph

The graph schema is version 2 and contains files, symbols, import edges, call references, resolution status, and summary counts. Local import resolution covers common Rust module paths, JavaScript/TypeScript relative imports, Python module imports, and Go module-aware local package imports. Go imports retain all source files in the resolved package directory. Third-party imports and unsupported forms remain explicitly unresolved or ambiguous.

Call references prefer enclosing-scope matches, then same-file matches, then unique repository-wide name matches. This remains heuristic rather than compiler name resolution. Overloading, receiver types, traits/interfaces, macros, dynamic imports, reflection, dependency injection, and runtime registration are not fully modeled.

### Feature boundaries

The boundary engine ranks candidate seed symbols using identifier and description keyword overlap. It follows resolved local imports and follows a cross-file call reference only when that target file is already connected by a resolved import. A unique repository-wide name match alone does not expand the capsule.

The current unit of extraction is a whole source file. This may include unrelated declarations in that file and may miss runtime-only dependencies. Inspect the warnings and unresolved edges before using a boundary.

### Feature Capsules

A capsule contains:
- Selected source files in `source/`.
- Nearby ancestor and root package manifests in `context/manifests/`.
- Root-level license/notice evidence in `licenses/`.
- A versioned `manifest.json` with file SHA-256 digests, source Git metadata when available, included symbols/files, unresolved imports, call references not followed into the boundary, and boundary warnings.

Capsule creation refuses to overwrite an existing directory, rejects source symlinks, and refuses output directories that overlap the source repository. `license_review_required` is always true: license evidence is copied, not legally interpreted or cleared.

The capsule is an extraction artifact, not a guaranteed standalone package. Its manifest hashes can be independently checked for internal consistency, but the current format does not provide a producer signature or authenticity attestation.

## Compatibility and integration

`composer compatibility` verifies a capsule, analyzes the target repository, inventories direct dependencies from Cargo, npm, Python, and Go manifests, and reports language alignment, normalized symbol-name collisions, test-file candidates, manifest warnings, and a compatibility level. The plan is advisory: symbol collisions, missing/different direct requirements, unresolved capsule imports, or capsule syntax errors force `ReviewRequired`; cross-language plans can return `RequiresAdapter` when no review blocker takes precedence. Direct dependency constraint strings are compared without solving versions or transitive dependencies, and normalized name collisions are not proof of semantic equivalence.

The generated adapter contract is a draft that lists candidate symbols and unresolved mapping questions. `composer integrate` stages only manifest-listed capsule files plus plan, verification, adapter-contract, and handoff documentation into a new `.composer/integrations/<capability-id>/` directory. It refuses to overwrite an existing destination and re-verifies the staged capsule before finalizing the directory.

`composer integrate` remains safe staging only. A separate opt-in `composer wire-rust` command implements a restricted single-file Rust source-level integration path, with a persistent entrypoint backup and fail-closed compatibility gates. General-purpose adapter generation, source rewrites, dependency solving and sandboxed verification remain unimplemented.

## Current limitations

- Public HTTPS GitHub acquisition only; shallow default-branch checkout.
- No private-repository credential broker.
- Supported syntax grammars do not imply complete semantic analysis; Rust module resolution remains approximate.
- Python package-relative edge cases, TypeScript path aliases/package exports, and compiler-grade symbol resolution remain incomplete.
- Dependency inventory covers selected direct manifest declarations; transitive resolution, lockfile solving, registry integrity, and vulnerability scanning are not implemented.
- SPDX expressions are syntax-checked only; license identifiers, policy compatibility, and legal approval are not established.
- Feature ranking is lexical, not embedding-based; feature boundaries are conservative file-level approximations.
- Adapter contracts require human review; general-purpose production adapter generation and multi-file/cross-language source patching are not implemented. The opt-in `wire-rust` path supports only standalone single-file Rust modules.
- Capsule hash verification does not attest producer authenticity.
- No untrusted repository build/test execution is enabled; sandboxed verification and bounded repair remain future work.

## Development checks

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

See [the architecture overview](docs/architecture/overview.md), [repository intelligence design](docs/architecture/repository-intelligence.md), and [the MVP plan](docs/architecture/mvp-plan.md).
