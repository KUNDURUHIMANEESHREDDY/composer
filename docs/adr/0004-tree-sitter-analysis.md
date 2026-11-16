# ADR-0004: Tree-sitter for initial multi-language source analysis

- Status: Accepted
- Date: 2026-11-16

## Context

Software Composer must understand repositories written in multiple languages without executing their source. A first useful analyzer needs to identify declarations, imports, and call expressions while remaining bounded and independently testable.

## Decision

Use Tree-sitter grammars for Rust, Python, JavaScript, TypeScript, TSX, and Go as the initial syntax layer. Normalize parser results into a language-independent repository graph.

Tree-sitter output is structural evidence only. The initial implementation uses explicit heuristic resolvers for common local imports and unique symbol-name call matches. External imports and unsupported forms remain unresolved or ambiguous.

Feature boundary calculation follows resolved import edges. It follows a call reference across files only when the target file is already connected by a resolved import; a unique repository-wide name match alone does not expand a capsule.

## Consequences

Positive:
- One parser interface supports multiple source languages.
- Syntax extraction can run without executing repository-controlled code.
- Unsupported semantic cases remain visible in graph evidence.

Trade-offs:
- Tree-sitter does not provide compiler-grade name or type resolution.
- Framework conventions, macros, dynamic imports, reflection, and runtime registration need additional analyzers.
- Feature boundaries are currently whole-file approximations and can include unrelated declarations.
- Language-server/compiler adapters remain necessary for higher-confidence extraction.
