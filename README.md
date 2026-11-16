# Software Composer

A Rust-first autonomous software composition agent. It aims to build applications by discovering, extracting, adapting, integrating, and verifying reusable capabilities from existing software repositories instead of rebuilding every capability from scratch.

> **Status: intelligence foundation implemented.** The project now has a Rust workspace, GitHub source acquisition, Tree-sitter parsing for six language variants, repository symbol/import analysis, heuristic capability seed recommendations, conservative feature-boundary analysis, and versioned Feature Capsule export. Autonomous multi-repository integration, semantic/ compiler-grade dependency resolution, and sandboxed builds remain under development.

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

The graph contains files, symbols, import edges, call references, resolution status, and summary counts. Local import resolution is currently implemented for common Rust module paths, JavaScript/TypeScript relative imports, and Python module imports. Third-party imports and unsupported forms remain explicitly unresolved or ambiguous.

Call references are linked in the repository graph only when an exact symbol name has one repository-wide match. This is a heuristic, not compiler name resolution. Same-file overloading, traits, macros, dynamic imports, reflection, dependency injection, and runtime registration are not fully modeled.

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

The capsule is an extraction artifact, not a guaranteed standalone package. It is not yet automatically adapted, built, or tested.

## Current limitations

- Public HTTPS GitHub acquisition only; shallow default-branch checkout.
- No private-repository credential broker.
- Supported syntax grammars do not imply complete semantic analysis.
- Go imports are parsed but are not mapped through `go.mod` yet.
- Rust resolution is approximate and does not understand the full Cargo module/crate model.
- Python package-relative cases such as `from . import sibling` remain unresolved.
- Dependency manifests are copied as evidence; transitive package dependencies are not yet calculated.
- Feature ranking is lexical, not embedding-based.
- Feature boundaries are conservative file-level approximations.
- License identification/policy evaluation and vulnerability scanning are not yet wired into capsule decisions.
- Capsules are not yet integrated into a target project.
- No untrusted repository build/test execution is enabled; sandboxed verification remains a separate milestone.

## Development checks

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

See [the architecture overview](docs/architecture/overview.md), [repository intelligence design](docs/architecture/repository-intelligence.md), and [the MVP plan](docs/architecture/mvp-plan.md).
