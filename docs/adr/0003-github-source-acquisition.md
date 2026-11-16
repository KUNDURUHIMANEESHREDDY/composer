# ADR-0003: GitHub HTTPS as the initial source-acquisition path

- Status: Accepted
- Date: 2026-11-16

## Context

The first product milestone needs a reproducible way to acquire source repositories while avoiding accidental execution of repository-controlled code.

## Decision

The initial acquisition crate accepts canonical HTTPS URLs on github.com only. It clones a shallow snapshot of the default branch into a unique workspace directory and records the resolved commit SHA and Git tree SHA.

Git is invoked with argument arrays through the process API, never through a shell. Recursive submodule fetching is disabled. Terminal prompts are disabled, and transport is restricted to HTTPS. Source code, build scripts, package lifecycle hooks, and tests are not executed during acquisition or manifest inventory.

The initial inventory recognizes common build manifests and uses a maximum traversal depth and file-count limit.

## Consequences

Positive:
- Acquisition is simple and reproducible at a pinned commit.
- A repository URL cannot inject shell syntax into the Git command.
- Submodules are not fetched implicitly.
- The analyzer can begin with manifest-only inventory before AST parsing exists.

Trade-offs and remaining work:
- Only public GitHub HTTPS repositories are supported.
- Clone time and disk usage are not yet bounded by configurable budgets.
- The default branch is shallow-cloned; arbitrary commit selection is not implemented.
- Manifest-based language detection is approximate.
- This is not a sandbox for building or testing repository code; all execution remains deferred to a separately isolated worker design.
