# Automatically select and verify reviewed external Rust capabilities

Composer can now combine its existing `cargo search` discovery with its existing `verify-external-rust-crate` isolated import. This path **does not generate third-party algorithm implementations**. It selects one unique entry from an independently reviewed catalog, confirms its package identity through crates.io, pins a reviewed version and delegates integration to the existing compiler-and-contract gate.

```powershell
cargo run -p composer-cli -- compose-external-rust "jaro string similarity" path/to/original-rust-target \
  --catalog examples/approved-external-capabilities.json \
  --output path/to/new-verified-target \
  --target-alias ported_jaro \
  --contract-test tests/composer_external_contract.rs \
  --registry-cache "$HOME/.cargo/registry" \
  --exclude-owner KUNDURUHIMANEESHREDDY \
  --license-reviewed
```

The target must be an independently reviewed standalone Rust crate, and it must already contain a contract test exercising the named `ported_jaro` API with independent expected results. This command does **not** write that contract. The reviewed catalog currently maps `jaro` to MIT-licensed `rapidfuzz/strsim-rs` and `luhn validation` to MIT-licensed `jeffcarp/luhn-rs`, both pinned to exact Git revision claims and published version strings. Users can extend their own catalog after reviewing upstream source and licensing.

## Fail-closed conditions

- Source/license review was not explicitly asserted; the JSON catalog is invalid, excessively large or a symlink
- No exact capability alias match, or more than one approved entry claims the same alias
- A source repository owner matches `--exclude-owner`
- Unpinned versions, invalid Git SHA, invalid package or source URL
- Cargo's registry search does not confirm the exact approved package name
- Source/target dependency conflict, invalid integration alias, missing independently authored contract or unsafe target file
- Offline Docker compilation or behavioral tests fail

No output is published in any of those cases. Source repository revision claims are **not** proven equivalent to the published .crate archive; Cargo verifies the registry artifact checksum separately. Registry descriptions are untrusted metadata and are not used as instructions. The original target is not modified.

## Evidence and scope

`tests/test_auto_external_composition_public.py` runs repeated Jaro integration plus a second external Luhn source and deliberate negative controls. The test is exercised by `.github/workflows/auto-reviewed-external.yml`.

This **is not open-world autonomous repository discovery**. It automatically chooses a uniquely approved source for a named capability and invokes existing verification. New repository APIs, trait/method adaptation, security evaluation, comprehensive semantic equivalence, and arbitrary dependency graph repair remain unsupported. Do not automatically approve a license or source merely because a search result appeared.
