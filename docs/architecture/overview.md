# Architecture Overview

## Product boundary

Software Composer accepts a natural-language project brief, converts it into a structured specification, discovers candidate source implementations, evaluates their suitability, extracts reusable capabilities, integrates them into a target project, and verifies the result.

The product is not a code generator with repository search bolted on. Repository intelligence, dependency closure, integration boundaries, source provenance, and verification are first-class subsystems.

## System layers

1. **Control plane** — API, CLI, orchestration, policies, workflow state, and metadata.
2. **Intelligence plane** — requirement decomposition, repository analysis, capability matching, compatibility reasoning, and repair planning.
3. **Composition plane** — acquisition, extraction, adapters, integration, and artifact assembly.
4. **Execution plane** — isolated builds, tests, static analysis, and runtime verification.
5. **Evidence plane** — source snapshots, dependency evidence, license findings, test results, and provenance.

The control plane must not execute untrusted repository code. Execution workers receive bounded jobs and return explicit artifacts and results.

## Reuse strategy

Choose the least invasive viable strategy:

1. Reuse a published package.
2. Reuse a self-contained module.
3. Isolate a tightly coupled feature behind a service or process boundary.
4. Transplant code only when the required dependency closure is understood and tests can verify it.
5. Implement the missing portion when reuse is unsafe, incompatible, or more expensive than replacement.

## Initial workspace boundaries

- `composer-core`: shared domain types and deterministic invariants.
- `composer-api`: transport layer; owns HTTP routes and request/response mapping.
- `composer-cli`: local developer interface.
- Future crates should be introduced only when a subsystem has a stable contract and a meaningful independent test boundary.

## Project specification

The initial `ProjectSpec` contains:

- A human-readable project name and description.
- One or more requested capabilities.
- Optional preferred implementation languages.

Capability IDs use lowercase kebab-case and are unique within a specification. Preferred languages are preferences, not hard restrictions.

The current API validates specifications only. Persistence and autonomous execution are deliberately not implied yet.

## Planned composition workflow

```text
Specification
  -> Requirement decomposition
  -> Candidate discovery
  -> Immutable source acquisition
  -> Repository analysis
  -> Capability boundary identification
  -> Dependency closure
  -> Feature Capsule
  -> Compatibility plan
  -> Integration plan
  -> Sandboxed integration
  -> Verification and repair
  -> Provenance-backed delivery
```

## Source of truth

Every acquired repository must be pinned to a commit SHA and snapshot digest. Branch names are discovery hints, not reproducible source identities.

Every integrated file should be traceable to one of:
- an upstream source file and immutable revision;
- an explicit transformation of upstream code;
- first-party adapter or glue code;
- newly implemented first-party code.

## Non-goals for the first milestone

- Automatic production deployment.
- Arbitrary cross-runtime service decomposition.
- Unrestricted execution of third-party scripts.
- A global public Feature Capsule registry.
- Full semantic analysis for every programming language.
