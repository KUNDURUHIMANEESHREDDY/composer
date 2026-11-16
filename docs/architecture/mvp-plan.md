# MVP Plan

## Goal

Prove the hardest product assumption: a reusable capability can be found, analyzed, extracted with its required dependencies, and integrated into a separate target project with evidence that it still works.

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

Acceptance evidence:
- A public repository was cloned successfully: serde-rs/serde.
- The command recorded commit e306b0c7c40ad21d5bdf52bcd20da513904682ae.
- The command recorded Git tree 0d60d815badb637ff2c556a160cab3aed85e605d.
- Seven Cargo manifests were discovered.
- No repository build, package, or test scripts were invoked.

## Milestone 2 — Repository inventory and language analysis

- Detect languages and build systems from manifests and lockfiles.
- Inventory source files while respecting ignore rules and size limits.
- Detect license and notice files.
- Produce a machine-readable repository inventory.
- Add Tree-sitter parsing for TypeScript/JavaScript and Rust.
- Identify files, modules, imports, exports, and tests.

## Milestone 3 — Dependency graph

- Define stable nodes and typed relationships.
- Record evidence and confidence for every inferred edge.
- Add manifest dependencies, imports, symbols, configuration keys, test references, and runtime services.
- Preserve unknown dependencies rather than claiming complete closure.

## Milestone 4 — Feature Capsule

- Define a versioned capsule manifest.
- Identify feature entry points and dependency closure.
- Classify dependencies as required, optional, replaceable, external, or unresolved.
- Copy only the minimum approved source set.
- Preserve required licenses and notices.
- Verify the capsule in isolation.

## Milestone 5 — Integration and verification

- Inspect target project architecture.
- Detect existing equivalent capabilities.
- Generate an integration plan and adapter contracts.
- Apply changes in a disposable target workspace.
- Run unit, contract, integration, and regression tests.
- Record transformations and results.

## First end-to-end demo

Compose a small application using two public repositories and one target template. Limit the experiment to a single language ecosystem and two capabilities. The demo must include the source commit, license evidence, extracted file list, dependency closure, integration diff, and test results.

## MVP non-goals

- Multi-cloud deployment.
- Multi-tenant SaaS.
- Global capability marketplace.
- Arbitrary language translation.
- Automatic resolution of ambiguous licensing.
