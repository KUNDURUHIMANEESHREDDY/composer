# ADR-0001: Rust-first core

- Status: Accepted
- Date: 2026-11-16

## Context

Software Composer performs repository acquisition, source analysis, dependency graph construction, filesystem transformations, workflow coordination, and sandbox orchestration. It must process untrusted and potentially large repositories while exposing predictable operational behavior.

## Decision

Use Rust for the control-plane backend, domain model, repository tooling, analysis orchestration, extraction, dependency resolution, integration, verification coordination, and CLI.

Use TypeScript for the web frontend. Language-specific analyzers may invoke compiler, language-server, or ecosystem tooling. Python is permitted as an optional adapter when it provides a material ecosystem advantage, but it is not a core runtime requirement.

## Consequences

Positive:
- Strong domain modeling and explicit error handling.
- Efficient concurrent repository processing.
- A portable native CLI and worker runtime.
- One primary backend language.

Trade-offs:
- Some AI and code-analysis integrations may have stronger Python libraries.
- Compiler-grade semantic analysis still requires language-specific tooling.
- Rust does not make execution of untrusted code safe; sandboxing remains mandatory.
