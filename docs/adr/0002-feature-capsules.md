# ADR-0002: Feature Capsules as the reuse artifact

- Status: Accepted
- Date: 2026-11-16

## Context

Copying a feature's apparent source files is insufficient when behavior depends on configuration, runtime services, database models, authentication, external APIs, or transitive code dependencies.

## Decision

Represent each extracted capability as a versioned Feature Capsule. A capsule contains source files, a manifest, dependency closure, runtime and configuration requirements, interface contracts, applicable license notices, source provenance, and verification results.

Dependencies must be classified as required, optional, replaceable, external, or unresolved. Unknown relationships must remain explicit.

## Consequences

Positive:
- Extraction results can be inspected and tested independently.
- Integrations can be reproduced and audited.
- Capsules can later be cached and reused across projects.

Trade-offs:
- Capsule construction requires substantial analysis.
- A capsule is not necessarily portable across frameworks or runtimes.
- Capsule schemas need explicit versioning and migration rules.
