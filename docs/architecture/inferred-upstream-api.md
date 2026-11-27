# Bounded automatic Rust API type inference from third-party Git source

Composer now reuses its **existing Tree-sitter Rust AST parser** to infer a narrowly supported type adapter. This avoids writing a new Rust parser or regenerating an algorithm.

## Example

A reviewed pinned external source exposes a public top-level function:

```rust
pub fn deunicode(s: &str) -> String { /* original upstream implementation */ }
```

Composer's existing Git import command may be used with `--infer-owned-string-signature` and `--target-alias transliterate` instead of requiring the reviewer to manually provide `--target-signature 'transliterate(String)->String'`.

```sh
cargo run --locked -p composer-cli -- verify-external-git-rust TARGET \
  --output VERIFIED_OUTPUT \
  --upstream-checkout PINNED_DEUNICODE_CHECKOUT \
  --upstream-repository https://github.com/kornelski/deunicode \
  --upstream-commit cfb8552fbbdf6d1f3f996ee4f2e78ec5e482bcef \
  --package deunicode \
  --public-symbol deunicode \
  --target-alias transliterate \
  --infer-owned-string-signature \
  --contract-test tests/composer_git_string_contract.rs \
  --registry-cache /path/to/pre-fetched/.cargo/registry \
  --git-cache /path/to/pre-fetched/.cargo/git \
  --license-reviewed
```

The resulting adapter is **only** a compiler-checked call into the real upstream implementation:

```rust
pub fn transliterate(arg0: String) -> String {
    composer_git_deunicode::deunicode(arg0.as_str())
}
```

The inference reads `src/lib.rs` from an exact Git commit verified against the original clean public GitHub checkout. It accepts at most 512 KiB of regular non-symlink source code, scans the public root-level Rust AST using the **preexisting Tree-sitter parser**, requires exactly one eligible function, 1-4 borrowed `&str` arguments, and a `String` result. The caller must explicitly choose the target-facing name and request an owned-string adapter. It refuses unsupported types, methods, modules, generics, malformed syntax, ambiguity, mismatched Git SHA, and simultaneous explicit and inferred signature flags. It never executes untrusted source during inference.

The target-owned behavior contract and license review remain mandatory. Compiler success alone is not accepted; Composer tests the actual source and staged target in its restricted offline Docker verifier (optionally using pinned upstream gVisor runsc) and validates the Cargo.lock Git revision before publishing. Existing verified output records the inferred signature and an inference flag.

## Proof and limits

`tests/test_pinned_git_string_adapter_public.py` now requires three independently tested deunicode-to-slugify Git transfers with the **inferred** String adapter. Wrong-but-compiling semantics, missing adaptation, unsafe explicit signatures, Git SHA mismatches, and unsupported inference must all fail without output. This is **not** semantic inference, open-ended automatic repository/API discovery, or verification of all possible behaviors. Support for optional lifetimes, generic trait APIs, borrowed return values, and arbitrary conversions requires separate reviewed upstream capabilities and benchmarks.
