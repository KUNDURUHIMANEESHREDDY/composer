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

The inference reads `src/lib.rs` from an exact Git commit verified against the original clean public GitHub checkout. It accepts at most 512 KiB of regular non-symlink source code, scans the public root-level Rust AST using the **preexisting Tree-sitter parser**, requires exactly one eligible function, 1–4 borrowed `&str` arguments, and an explicitly supported `String`, `bool`, or numeric scalar result (`f32`, `f64`, `u8`–`u64`, `i8`–`i64`). No conversions of result values are invented. The caller must explicitly choose the target-facing name and request an owned-string adapter. It refuses unsupported types, methods, modules, generics, malformed syntax, ambiguity, mismatched Git SHA, and simultaneous explicit and inferred signature flags. It never executes untrusted source during inference.

The target-owned behavior contract and license review remain mandatory. Compiler success alone is not accepted; Composer tests the actual source and staged target in its restricted offline Docker verifier (optionally using pinned upstream gVisor runsc) and validates the Cargo.lock Git revision before publishing. Existing verified output records the inferred signature and an inference flag.

## Numeric and predicate output reuse

The exact same upstream-lookup and compiler-checked wrapper also supports a scalar return. For example, the independent MIT source `rapidfuzz/strsim-rs` exports `pub fn jaro(a: &str, b: &str) -> f64`. With an explicit reviewed source symbol `jaro`, target alias `ported_jaro`, and `--infer-owned-string-signature`, Composer infers:

```rust
pub fn ported_jaro(arg0: String, arg1: String) -> f64 {
    composer_git_strsim::jaro(arg0.as_str(), arg1.as_str())
}
```

It never implements Jaro internally. `tests/test_pinned_git_numeric_api_inference.py` checks three independent transfers into unrelated `ijt/trigram`, including coexistence with an older dependency version. A type-compatible but semantically different upstream `jaro_winkler` must fail target-owned Jaro assertions. Generic source APIs and missing adapters must also fail without publication. Static type inference cannot determine whether `jaro` is the intended behavior: the source name, reviewed Git commit, license, and expected behaviors remain caller-supplied.

## Step 1: broader compiler-verified typed API inference

Composer reuses its existing **Tree-sitter Rust parser**, Cargo's pinned Git dependency resolver, and the Rust compiler; it does not invent a Rust parser or rewrite upstream algorithms. A single reviewed public top-level source function can now be adapted when its inputs consist of concrete scalar/boolean/char types, owned `String`, and immutable borrowed `&str` in any supported combination (maximum four parameters), with a supported scalar, `String`, or unit result. The only value conversion Composer writes is target-owned `String.as_str()` for an upstream `&str`; all other arguments pass through unchanged without casts. A no-argument source function is also supported.

For example, the unmodified third-party Git implementation `rapidfuzz/strsim-rs` exports `levenshtein(&str, &str) -> usize`. The existing command accepts `--infer-typed-signature` as a readable alias for the original `--infer-owned-string-signature`, and automatically produces this source-checked wrapper:

```rust
pub fn ported_levenshtein(arg0: String, arg1: String) -> usize {
    composer_git_strsim::levenshtein(arg0.as_str(), arg1.as_str())
}
```

The independently authored target contract must still compare observed distances with known expectations. The CI benchmark tests the `usize` adapter three times on the unrelated pinned `ijt/trigram` target and separately checks that the same-signature but semantically different `osa_distance` implementation is **refused**, with no publication or modification to the original target. Existing `f64` Jaro and transliteration imports must remain green.

**Boundary:** This is automatic **type-shape inference**, not unrestricted cross-project semantic understanding. Callers still select a reviewed Git repo/commit, source function and target alias, review the license and supply an independent behavioral contract. Arbitrary generics, references in return types, traits, impl methods, user-defined types, modules, macros and automatic meaning-preserving cross-language rewrites are not asserted. The integration fails closed on unsupported signatures; manual reviewed integrations remain available. The types are drawn from bounded, clean pinned Git source and then checked by actual Cargo compilation in the restricted offline verifier.

## Step 2: behavior-verified choice among reviewed Git functions

**Source-symbol discovery is a different question from semantics.** Composer can now choose **one** implementation out of 2–5 explicitly reviewed public symbols from the **same** clean SHA-pinned Git checkout. The CLI uses the existing `verify-external-git-rust --infer-typed-signature` path with additional `--alternative-public-symbol` arguments:

```text
--public-symbol osa_distance
--alternative-public-symbol levenshtein
--target-alias ported_levenshtein
--infer-typed-signature
```

Composer does **not** claim that AST signatures or names prove meaning. Each candidate runs through the exact existing Cargo resolver and independent target-owned tests, separately in temporary output directories. The requested output is published only if **exactly one** candidate passes. Zero verified candidates and multiple passing candidates both fail closed. This also prevents a weak behavior test from silently selecting one of two equivalent-looking implementations.

The repeated real Git benchmark exercises upstream `strsim::osa_distance` versus `strsim::levenshtein` (both `(&str,&str)->usize`), into the independent `ijt/trigram` target. Target assertions on transposed strings reject OSA while accepting Levenshtein. Additional negative cases use deliberately weak all-equal-input assertions (both candidates pass) and wrong candidates (neither passes); **both must refuse publication**. The algorithms are reused unchanged from the third-party upstream. This is **bounded test-based semantic selection, not unrestricted semantic equivalence**: callers must still choose reviewed alternatives and write sufficiently discriminating independent behavior contracts. Ambiguity is a valid safety outcome, not a missing check.

## Proof and limits

`tests/test_pinned_git_string_adapter_public.py` now requires three independently tested deunicode-to-slugify Git transfers with the **inferred** String adapter. Wrong-but-compiling semantics, missing adaptation, unsafe explicit signatures, Git SHA mismatches, and unsupported inference must all fail without output. This is **not** semantic inference, open-ended automatic repository/API discovery, or verification of all possible behaviors. Support for optional lifetimes, generic trait APIs, borrowed return values, and arbitrary conversions requires separate reviewed upstream capabilities and benchmarks.
