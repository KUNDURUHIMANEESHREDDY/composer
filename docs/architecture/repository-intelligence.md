# Repository Intelligence and Feature Capsules

## Scope

This document describes the current implementation in `composer-intelligence`. It deliberately distinguishes implemented heuristics from future compiler-grade or runtime-aware capabilities.

## Pipeline

```text
Repository checkout
  -> bounded source-file inventory
  -> language identification
  -> Tree-sitter parse
  -> declaration and import extraction
  -> cross-file import resolution
  -> call-name reference linking
  -> repository graph
  -> capability seed recommendations
  -> conservative file-level dependency closure
  -> Feature Capsule
```

## Supported grammars

| Language | Tree-sitter grammar | Current extracted constructs |
| --- | --- | --- |
| Rust | tree-sitter-rust | Functions, methods, structs, enums, traits, type aliases, modules, constants, macros, `use` and `mod` |
| Python | tree-sitter-python | Functions, methods, classes, top-level assignments, imports and from-imports |
| JavaScript | tree-sitter-javascript | Functions, classes, methods, top-level function-valued declarations, imports and re-exports |
| TypeScript | tree-sitter-typescript | JavaScript constructs plus interfaces and type aliases |
| TSX | tree-sitter-typescript TSX grammar | TypeScript constructs in TSX source |
| Go | tree-sitter-go | Functions, methods, struct/interface type declarations, aliases, imports |

The parser tracks syntax error counts per source file. Syntax trees are structural evidence; parsing does not prove that a program compiles or behaves correctly.

## Inventory limits

- Maximum 50,000 source files.
- Maximum 1,000,000 bytes per source file.
- Maximum traversal depth 12.
- Symlinks and common dependency/build/hidden directories are skipped.
- Files must be valid UTF-8 to be parsed.

The current analyzer fails the operation when the source-file cap is exceeded; file-size and UTF-8 failures are counted as skipped files.

## Graph schema

The graph has schema version 1.

### Nodes

- Source files.
- Symbols, with byte and line ranges.
- Import dependencies.
- Call references.

### Relationships and evidence

- Imports are classified as `local`, `unresolved_or_external`, or `ambiguous`.
- Call references are classified as `unique_name_match`, `ambiguous_name_match`, or `unresolved`.
- Symbol IDs are deterministic for an unchanged path, node kind, name, and byte start within the same source layout.

A unique call-name match is not a compiler-verified reference. It can be incorrect when a repository has methods or functions with the same name in different scopes.

## Import resolver coverage

- JavaScript/TypeScript/TSX: relative path imports with common source extensions and index files.
- Python: dotted absolute module imports and common package-relative imports.
- Rust: common `mod` declarations and approximate crate/self module paths.
- Go: imports are extracted but not resolved yet because module-path and package-directory semantics require reading and applying the `go.mod` module path.
- External package imports remain unresolved. This is intentional; the resolver does not invent local files for package names.

## Capability matching

`recommend_feature_seeds` tokenizes the capability identifier, description, symbol name, source path, and symbol kind. It splits camelCase and underscore identifiers, filters a small stop-word list, and ranks candidates by keyword overlap.

This is a discovery heuristic only. It does not use LLM reasoning, embeddings, API semantics, or behavioral evidence. Candidates should be reviewed before capsule creation.

## Feature boundary algorithm

1. Validate the capability ID and seed symbol IDs.
2. Add seed files and seed symbols.
3. For every included file, follow all resolved local import edges.
4. Follow a call reference to another file only when a resolved local import already connects that target file to the boundary; a unique repository-wide name match alone does not expand the boundary.
5. Include complete files, then include all declarations discovered in those files.
6. Retain unresolved/ambiguous imports and all call references that were not followed into the boundary, including unique-name matches without an import edge.

The approach is conservative at file granularity: unrelated code in the same file can enter a boundary. It can also over-approximate due to repository-wide name matching and under-approximate due to dynamic behavior.

The warnings emitted in every boundary are part of the artifact contract and should not be discarded by callers.

## Feature Capsule format

Capsule schema version 1 is written to a new output directory:

```text
capsule/
├── manifest.json
├── source/
│   └── <included repository source files>
├── context/
│   └── manifests/
│       └── <nearest and root package manifests>
└── licenses/
    └── <root-level LICENSE/COPYING/NOTICE/AUTHORS/COPYRIGHT evidence>
```

Each copied file records:
- Original relative path.
- Capsule-relative path.
- File kind.
- Byte size.
- SHA-256 digest.

The capsule manifest records Git origin URL (credentials stripped), commit/tree IDs when the checkout has Git metadata, seed and included symbol IDs, unresolved dependency evidence, and boundary warnings.

The current manifest copies the nearest known ancestor package manifests and root-level license evidence. It does not yet parse package dependencies into a transitive package lock closure, copy file-level license headers, identify an SPDX license conclusively, or generate a legal compliance verdict.

## Safety properties

- Analysis does not execute repository source.
- Symlinked source paths are skipped by inventory and rejected during capsule copy.
- Capsule source paths must be relative and consist only of normal path components.
- Output path is normalized and resolved through its existing filesystem ancestor before source-overlap checks.
- Output cannot overlap the source repository.
- Existing output directories are never overwritten.
- Capsule writes are staged in a unique temporary directory and renamed into place after manifest creation.

## Known gaps and next work

1. Go module-aware import resolution.
2. Rust Cargo crate/module-aware resolution and conditional compilation.
3. Python namespace packages and all relative import forms.
4. JavaScript package exports, tsconfig path aliases, and bundler resolution.
5. Scope-aware symbol linking and reference confidence.
6. Language-specific package dependency graph and lockfile analysis.
7. File-level license evidence and SPDX expression parsing.
8. Feature-level test discovery and extraction.
9. Capsule self-validation and reproducible build verification in a sandbox.
10. Adapter generation and integration into a target workspace.
