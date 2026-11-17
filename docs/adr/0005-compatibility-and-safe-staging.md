# ADR-0005: Advisory compatibility planning and non-destructive capsule staging

- Status: Accepted
- Date: 2026-11-16

## Context

Feature Capsules are traceable source extraction bundles, but extraction alone does not establish that a capability can be safely used by a target project. The next milestone needs to verify capsule integrity, inspect the target and its declared dependencies, expose compatibility risks, and provide a repeatable handoff without executing untrusted repository code or silently modifying user source.

The current analyzers are heuristic. Direct dependency manifests are not a package solver, syntax-based symbol matches are not compiler-grade semantic equivalence, and SPDX expression parsing is not legal review. Integration must preserve these distinctions in its output.

## Decision

Create a separate `composer-integration` crate with four responsibilities:

1. Verify capsule manifest paths, regular-file status, sizes, and SHA-256 hashes before use.
2. Inventory supported direct dependency declarations and analyze the target repository's languages, symbols, package managers, and test-file conventions.
3. Produce an advisory compatibility plan and draft adapter contract that retain warnings and unresolved human decisions.
4. Stage only manifest-listed capsule files and supporting evidence under a new target-local `.composer/integrations/<capability-id>/` namespace.

Staging refuses traversal and symlink paths, refuses an existing destination, writes to a temporary sibling, and re-verifies the staged capsule before publishing the directory. It does not overwrite target source, install dependencies, invoke package scripts, build, or run tests.

The CLI exposes `verify-capsule`, `compatibility`, and `integrate` commands. Capsule hash consistency is not treated as producer authenticity; a future signed attestation remains separate work.

## Consequences

Positive:
- Capsule integrity is checked independently at the integration boundary.
- Compatibility findings and adapter questions are inspectable machine-readable artifacts.
- The first integration milestone can demonstrate a real target-aware workflow without executing untrusted code or mutating application source.
- The intelligence and integration domains have a stable crate boundary.

Trade-offs and remaining work:
- Dependency analysis covers selected direct manifest declarations only; version solving, lockfiles, and transitive closure remain future work.
- License expressions are syntax-checked only; identifier validation, policy evaluation, and legal approval are not provided.
- Compatibility levels and symbol collision findings are heuristic and require human review.
- Adapter contracts are drafts, not generated production adapters.
- Staging is not application integration. Sandboxed builds/tests, source patches, rollback, and bounded repair remain future milestones.
