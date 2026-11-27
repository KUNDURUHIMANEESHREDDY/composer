# Type-checked external Rust API adapters

Use Composer to **reuse a reviewed upstream public Rust function** with an owned-`String` target API, even when upstream expects borrowed `&str`. Composer generates only a small typed glue function; it never copies or regenerates the upstream algorithm.

Example for reviewed BSD-3-Clause `kornelski/deunicode` into a standalone Rust library with independent tests:

```sh
cargo run --locked -p composer-cli -- compose-external-rust \
  "unicode to ascii transliteration" path/to/target \
  --catalog examples/approved-external-capabilities.json \
  --output path/to/new-target \
  --target-alias transliterate \
  --target-signature 'transliterate(String)->String' \
  --contract-test tests/composer_external_transliteration_contract.rs \
  --registry-cache "$HOME/.cargo/registry" \
  --exclude-owner KUNDURUHIMANEESHREDDY \
  --license-reviewed
```

The target's `tests/` file must be an independently authored, pre-existing executable contract that calls `transliterate(String)` and asserts exact expected values. With this reviewed signature, Composer generates the bounded shim:

```rust
pub fn transliterate(arg0: ::std::string::String) -> String {
    deunicode::deunicode(arg0.as_str())
}
```

The implementation still comes entirely from the pinned, externally published crate. The generator accepts only an existing public symbol, 1–4 `String` inputs, an allowed primitive/`String` return type, and a matching target function name. It rejects arbitrary Rust expressions, unapproved target signatures, and unsupported APIs. Rust must compile the actual upstream call and the independent behavioral suite must pass inside Composer's offline, non-root Docker verifier. **No output is published after any failure.**

The prior direct `pub use upstream::function as alias` path remains unchanged when `--target-signature` is absent. The deliberately mismatched no-adapter case is included in the benchmark; compilation failure is a rejection rather than a success.

## New held-out source→target benchmark

`tests/test_unseen_owned_string_adaptation.py` pins `kornelski/deunicode` (BSD-3-Clause) and unrelated `mattgathu/slugify` (MIT), checks three fresh successful integrations, rejects a wrong-but-compilable transliteration expectation, rejects injected API contracts and verifies no output is published on an unadapted type mismatch. Both original checkouts remain unmodified. The older target uses Rust 2015, so its integration tests require `extern crate slugify;` in addition to `use slugify::transliterate;`.

This is bounded *reviewed* API adaptation, not automatic signature inference, arbitrary trait support or safe-to-execute certification. Pinned Git commit metadata is not cryptographically matched to the crates.io archive; its Cargo.lock registry checksum is checked separately. Docker is not a hardened VM isolation boundary, and license review is a human attestation. This integration does not solve version conflicts itself; independent PR #19 separately develops candidate fallback and dependency-version isolation.