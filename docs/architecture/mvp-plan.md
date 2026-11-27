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
- [x] Go module-aware package resolution for local imports, including all source files in a package directory.
- [ ] Rust Cargo-aware module/crate resolution.
- [ ] Python namespace package and relative-import completeness.
- [ ] TypeScript tsconfig paths, package exports, and bundler resolution.
- [x] Heuristic scope-aware symbol resolution, preferring enclosing scope and same-file matches before unique repository-wide matches.

## Milestone 4 — Feature boundaries and capsules (initial implementation complete)

- [x] Keyword-based seed symbol recommendations.
- [x] Conservative file-level dependency closure.
- [x] Retain unresolved and ambiguous edges as warnings.
- [x] Feature Capsule schema version 1.
- [x] Copy included source files, nearest manifests, and root license evidence.
- [x] SHA-256 digest for each capsule file.
- [x] Source Git URL/commit/tree metadata when available.
- [x] Refuse existing output directories and source/output overlap.
- [ ] Resolve transitive package dependency closure and lockfile versions.
- [ ] Detect file-level license headers and evaluate SPDX identifiers/compatibility policy.
- [x] Independently verify capsule paths, sizes, and SHA-256 digests against the manifest.
- [ ] Add producer-signed capsule provenance attestation and registry.

A Feature Capsule remains a traceable extraction bundle, not a guaranteed standalone package. Hash verification establishes internal consistency, not producer authenticity or legal clearance.

## Milestone 5 — Compatibility and integration (static planning and safe staging implemented)

- [x] Inspect target project languages, source/symbol inventory, package managers, and test-file conventions.
- [x] Detect normalized symbol-name collisions as review findings; semantic equivalence detection remains future work.
- [x] Compare declared direct dependencies and report exact, missing, and differing constraints.
- [x] Generate a compatibility plan and draft adapter contract with unresolved mapping questions.
- [x] Stage verified capsule files and plan evidence into a new target-local `.composer/integrations/<capability-id>/` namespace without overwriting target source.
- [x] Re-verify staged capsule contents and manifest hash before publishing the staging directory.
- [x] Opt-in single-file Rust module wiring with backup, collision checks, and explicit review assertion (narrow first source-level slice).
- [x] Verified-copy Rust integration: conservative direct dependency adoption, offline Cargo lockfile resolution and offline Docker tests with limited resources and no network (narrow first sandbox slice).
- [x] Bounded composition of 2–5 distinct reviewed single-file Rust capsules with isolated tests after every stage, aggregate evidence and all-or-nothing output publishing.
- [x] Reconcile overlapping version-string Cargo requirements conservatively, with exact selected lockfile version checked against both requirements.
- [x] Limited Docker hardening: inspect and use a fixed local image ID, read-only root, network/IPC isolation, unprivileged UID, no extra Linux capabilities, resource and timeout limits.
- [x] One bounded automatic repair for explicitly missing/stale Cargo lockfiles, no repeated application test retries.
- [ ] General-purpose adapters, multi-file extraction/rewrites and cross-language integration.
- [ ] Full dependency graph solving, safe incompatible-version migrations and offline registry integrity/provenance policy.
- [ ] Independent VM-grade sandbox hardening, hostile-input testing and supply-chain verification.
- [ ] General autonomous code repair with typed diagnosis, risk-aware patching, bounded retries and end-to-end diff evidence.

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
