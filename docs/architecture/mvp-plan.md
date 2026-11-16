# MVP Plan

## Goal

Prove that a capability can be found in an existing repository, analyzed, extracted with its visible dependencies, and eventually integrated into a separate target project with evidence that it works.

## Milestone 0 — Foundation (complete)

- [x] Rust workspace.
- [x] Shared project specification and deterministic validation.
- [x] HTTP validation endpoint.
- [x] CLI validation command.
- [x] Workspace formatting, lint, and test commands.

## Milestone 1 — Repository acquisition (initial slice complete)

- [x] Accept HTTPS GitHub repository URLs.
- [x] Validate owner/repository identity and reject nested GitHub paths.
- [x] Clone into a unique workspace path without invoking a shell.
- [x] Record the resolved commit SHA and Git tree SHA.
- [x] Disable recursive submodule initialization.
- [x] Disable terminal prompts and restrict Git transport to HTTPS.
- [x] Inventory common build manifests with depth and file-count bounds.
- [x] Add URL validation and manifest inventory unit tests.
- [x] Exercise the acquisition CLI against a real public repository.
- [ ] Add configurable clone timeout, disk budget, and cancellation.
- [ ] Add repository-size checks and a stable snapshot artifact.
- [ ] Add private repository support through an explicit credential broker.

## Milestone 2 — Repository inventory and syntax analysis (initial implementation complete)

- [x] Bounded source file traversal.
- [x] Detect Rust, Python, JavaScript, TypeScript, TSX, and Go source files.
- [x] Add Tree-sitter grammar adapters for those languages.
- [x] Extract selected declarations, imports, and call expressions.
- [x] Report syntax errors and skipped files.
- [ ] Expand grammar and framework-specific extraction coverage.
- [ ] Add language-server/compiler adapters for deeper semantic resolution.

## Milestone 3 — Symbol and import graph (initial implementation complete)

- [x] Versioned graph model.
- [x] Symbol IDs with source spans.
- [x] Import relationships with local, unresolved, and ambiguous states.
- [x] Call references with explicit name-match resolution states.
- [x] Local import resolution for common Rust, Python, and JS/TS patterns.
- [ ] Go module-aware package resolution.
- [ ] Rust Cargo-aware module/crate resolution.
- [ ] Python namespace package and relative-import completeness.
- [ ] TypeScript tsconfig paths, package exports, and bundler resolution.
- [ ] Scope-aware semantic symbol resolution.

## Milestone 4 — Feature boundaries and capsules (initial implementation complete)

- [x] Keyword-based seed symbol recommendations.
- [x] Conservative file-level dependency closure.
- [x] Retain unresolved and ambiguous edges as warnings.
- [x] Feature Capsule schema version 1.
- [x] Copy included source files, nearest manifests, and root license evidence.
- [x] SHA-256 digest for each capsule file.
- [x] Source Git URL/commit/tree metadata when available.
- [x] Refuse existing output directories and source/output overlap.
- [ ] Parse dependency manifests into transitive package dependency closure.
- [ ] Detect file-level license headers and parse SPDX expressions.
- [ ] Add feature-specific tests and independent capsule verification.
- [ ] Add capsule provenance attestation and registry.

Important: a Feature Capsule is currently a traceable extraction bundle, not a guaranteed standalone package or integration-ready module.

## Milestone 5 — Compatibility and integration

- [ ] Inspect target project architecture.
- [ ] Detect existing equivalent capabilities.
- [ ] Generate an integration plan and adapter contracts.
- [ ] Apply changes in a disposable target workspace.
- [ ] Run unit, contract, integration, and regression tests.
- [ ] Record transformations and results.

## Milestone 6 — Autonomous engineering loop

- [ ] Add durable workflow orchestration and typed agent contracts.
- [ ] Discover candidates based on the user project specification.
- [ ] Evaluate repository suitability, maintenance, security, and licensing evidence.
- [ ] Select feature seeds and request approval only at policy gates.
- [ ] Extract, adapt, integrate, verify, and boundedly repair.
- [ ] Produce an auditable delivery report.

## First end-to-end demo

Compose a small application using two public repositories and one target template. Limit the experiment to a single language ecosystem and two capabilities. The demo must include source commits, license evidence, extracted file list, dependency closure, integration diff, and test results.

## MVP non-goals

- Multi-cloud deployment.
- Multi-tenant SaaS.
- Global public capsule marketplace.
- Arbitrary language translation.
- Automatic legal approval of ambiguous licensing.
