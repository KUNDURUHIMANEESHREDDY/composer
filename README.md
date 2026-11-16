# Software Composer

A Rust-first autonomous software composition agent. It aims to build applications by discovering, extracting, adapting, integrating, and verifying reusable capabilities from existing software repositories instead of rebuilding every capability from scratch.

> **Status: early foundation.** The current implementation contains a Rust workspace, project-specification validation, a local API, a CLI, and a GitHub repository acquisition/manifest-inventory command. Semantic repository analysis, feature extraction, compatibility planning, and integration are not implemented yet.

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
| `crates/repository` | GitHub acquisition and basic repository inventory |
| `docs/architecture` | System design and contracts |
| `docs/adr` | Architecture decision records |
| `examples` | Example project specifications |

## Requirements

- Rust stable (edition 2024; minimum Rust 1.85)
- Git 2.x

## Quick start

Validate the example project specification:

```powershell
cargo run -p composer-cli -- validate-spec examples/project-spec.json
```

Inspect a public GitHub repository:

```powershell
cargo run -p composer-cli -- inspect https://github.com/serde-rs/serde
```

The inspect command clones a shallow snapshot under `.composer/sources`, disables recursive submodule fetching, records the commit and Git tree IDs, and inventories common build manifests. It does not run the repository's build scripts or tests.

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

## Development checks

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Current API

- `GET /health` — process health.
- `POST /v1/specifications/validate` — validate and normalize a project specification.

The API is currently stateless. A successful validation does not yet create or persist a project or start an agent workflow.

## Current repository-inspection limits

- Public HTTPS GitHub repositories only.
- Shallow clone of the default branch.
- No recursive submodule fetching.
- Manifest inventory is bounded by depth and file count.
- Language detection is manifest-based and approximate.
- No AST or dependency graph analysis yet.
- No repository code is executed during acquisition.

## Roadmap

1. Repository acquisition and immutable snapshot metadata.
2. Repository inventory and language-specific analysis.
3. Dependency and symbol graph.
4. Feature extraction boundaries and Feature Capsules.
5. Compatibility planning and adapters.
6. Sandboxed integration and verification.
7. Workflow orchestration, provenance, and user-facing progress.

See [the architecture overview](docs/architecture/overview.md) and [the MVP plan](docs/architecture/mvp-plan.md).
