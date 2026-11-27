# Software Composer

A Rust-first autonomous software composition agent. It aims to build applications by discovering, extracting, adapting, integrating, and verifying reusable capabilities from existing software repositories instead of rebuilding every capability from scratch.

> **Status: repository intelligence, safe capsule staging, and restricted Rust integration verification implemented.** The project supports GitHub acquisition, six-language Tree-sitter analysis, a versioned repository graph, conservative Feature Capsules, direct dependency inventories, compatibility plans, and isolated staging. An opt-in single-file Rust workflow can now wire reviewed code, adopt version-only direct dependencies, solve an offline Cargo lockfile, and run Docker-isolated tests against a new target copy. General-purpose adapter generation, complex dependency solving, hostile-code isolation hardening, and autonomous repair remain under development.

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

Run **offline, isolated build verification** on a **new copy** of a reviewed Rust target:

```powershell
docker pull rust:1.99-slim # one-time setup; image is never pulled during a verification run
cargo run -p composer-cli -- verify-rust .composer/capsules/authentication path/to/target --output path/to/new-verified-target --license-reviewed --registry-cache "$HOME/.cargo/registry"
```

The registry-cache argument is optional when no external crates are needed and must point to a Cargo registry directory. The cache is mounted **read-only**. For the compiler-verified single-file Rust path, Composer conservatively selects direct version-string dependencies referenced in the extracted source file; unrelated upstream development/optional dependencies are not transplanted. This lexical approximation requires successful isolated compilation and tests before publishing. Different source/target version constraints are preserved and accepted **only** when the offline resolved lockfile chooses a version satisfying both (otherwise verification fails without publishing an output). Composer checks the lockfile and can regenerate a missing/stale lockfile once on the specific Cargo lockfile error; it does not blindly retry failing application tests. The original target is unchanged; successful results include a resolved `Cargo.lock`, executable test evidence and a `composer-build-evidence.json` file.

### Compiler-verified multi-file Rust modules (bounded first slice)

The existing verify-rust command now accepts a Rust Feature Capsule with **2–32 source files** under one standalone source crate's root src/ directory. Composer preserves the module tree inside src/composer_<capability>/, adapts AST-identified crate:: references to the private namespace, supplies missing file-backed module declarations, and selects referenced version-string dependencies across **all** included source files. It then compiles and runs tests in the existing restricted, offline Docker stage before publishing a **new** target directory. Source/license provenance remains attached. The unverified wire-rust command still rejects multi-file capsules.

For example, source files src/lib.rs, src/math.rs and src/math/util.rs become src/composer_nested_calculator/mod.rs, math.rs and math/util.rs, with crate::math rewritten to crate::composer_nested_calculator::math. The repository analyzer follows mod util from math.rs into math/util.rs (rather than looking for a sibling src/util.rs).

This is a deliberately limited Rust module-graph integration, **not** arbitrary code rewriting: the source files must be covered by the verified capsule, compile in the target Rust crate, and fit the bounded file and size limits. Unsupported path-sensitive constructs, arbitrary build scripts, workspace/package boundaries, non-string dependency forms, macro-generated imports, cross-language adapters and untrusted execution outside Docker are not supported. The original target and upstream repositories are not edited.

Compose **two to five independently reviewed Rust capsules** from different repositories into one target, with isolation and tests after *each* change:

```powershell
cargo run -p composer-cli -- verify-rust-batch --capsule path/to/capsule-a path/to/capsule-b --target path/to/target --output path/to/verified-target --license-reviewed --registry-cache "$HOME/.cargo/registry"
```

If any stage fails, Composer removes intermediate copies and never publishes the requested output. A successful run includes `composer-batch-evidence.json` containing each step's independent verification. Verified outputs also preserve upstream license notices under `third_party_licenses/` and pinned source attribution in `composer-source-attributions.json`; root target license files are retained. This is a bounded multi-repository Rust workflow, **not** general cross-language or arbitrary multi-file source transplantation.

Verification uses an existing local `rust:1.99-slim` image by its inspected content ID, with network disabled, an isolated IPC namespace, a read-only container root, an unprivileged UID, dropped Linux capabilities, no-new-privileges, CPU/memory/file-descriptor/process limits and a 240-second timeout. The image ID is recorded for auditability but does not certify image provenance. **Docker is not production-grade hostile-code isolation**; use an isolated microVM/hardened runner before admitting untrusted third-party code. Cargo workspaces, cross-language adapters, path/git dependencies, complex dependency migration, compiler-grade source graph discovery and general autonomous repair remain unsupported. Independent security and license review is required.

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

`composer integrate` remains safe staging only. The opt-in `composer wire-rust` command provides restricted single-file Rust wiring with a persistent entrypoint backup. `composer verify-rust` extends it with conservative missing-dependency adoption, offline Cargo lockfile solving, and Docker-isolated compilation/tests on a newly generated target copy; it publishes the copy and evidence only after success. General-purpose adapter generation and broader dependency solving remain unimplemented.

## Current limitations

- Public HTTPS GitHub acquisition only; shallow default-branch checkout.
- No private-repository credential broker.
- Supported syntax grammars do not imply complete semantic analysis; Rust module resolution remains approximate.
- Python package-relative edge cases, TypeScript path aliases/package exports, and compiler-grade symbol resolution remain incomplete.
- Dependency inventory covers selected direct manifest declarations. Only `verify-rust` performs offline transitive Cargo resolution for supported standalone targets with pre-cached crates; general package solving, registry integrity and vulnerability scanning are not implemented.
- SPDX expressions are syntax-checked only; license identifiers, policy compatibility, and legal approval are not established.
- Feature ranking is lexical, not embedding-based; feature boundaries are conservative file-level approximations.
- Adapter contracts require human review; general-purpose production adapter generation and multi-file/cross-language source patching are not implemented. The opt-in `wire-rust` path supports only standalone single-file Rust modules.
- Capsule hash verification does not attest producer authenticity.
- `verify-rust` runs untrusted code only inside a restricted Docker container. This is not VM-grade isolation; general-purpose hardened build infrastructure, adversarial testing and bounded repair remain future work.

## Development checks

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

See [the architecture overview](docs/architecture/overview.md), [repository intelligence design](docs/architecture/repository-intelligence.md), [sandbox controls and production hardening gaps](docs/architecture/verification-hardening.md), [pinned public-repository validation](docs/validation/pinned-public-rust-repositories.md), and [the MVP plan](docs/architecture/mvp-plan.md).
